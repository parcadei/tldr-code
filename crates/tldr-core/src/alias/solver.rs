//! Fixed-Point Solver for Alias Analysis
//!
//! This module implements the worklist-based fixed-point iteration algorithm
//! for Andersen's subset-based points-to analysis. The solver propagates
//! points-to sets through constraints until convergence.
//!
//! # Algorithm
//!
//! 1. Initialize points-to sets from Alloc constraints
//! 2. Add all variables to worklist
//! 3. While worklist not empty and iterations < MAX_ITERATIONS:
//!    - Pop variable v from worklist
//!    - For each constraint involving v:
//!      - Apply propagation rule
//!      - If points-to set changed, add affected vars to worklist
//! 4. Compute may_alias: vars with overlapping points-to sets
//! 5. Compute must_alias: vars with identical singleton points-to sets
//! 6. Compute transitive closure for must_alias (TIGER-6)
//!
//! # TIGER Mitigations
//!
//! - **TIGER-2**: Tracks last-changed variables for debugging if MAX_ITERATIONS hit
//! - **TIGER-6**: Computes transitive closure for must_alias relationships

use std::collections::{HashMap, HashSet, VecDeque};

use super::constraints::{Constraint, ConstraintExtractor};
use super::types::{AbstractLocation, AliasError, AliasInfo, MAX_FIELD_DEPTH};

// =============================================================================
// Constants
// =============================================================================

/// Maximum number of fixed-point iterations before giving up.
///
/// This prevents infinite loops in cyclic constraint graphs.
/// The value of 100 is sufficient for most practical programs.
pub const MAX_ITERATIONS: usize = 100;

// =============================================================================
// Solver
// =============================================================================

/// Fixed-point solver for Andersen's points-to analysis.
///
/// The solver maintains points-to sets for each variable and iteratively
/// propagates values through constraints until a fixed point is reached.
///
/// # Example
///
/// ```rust,ignore
/// use tldr_core::alias::{ConstraintExtractor, AliasSolver};
///
/// let extractor = ConstraintExtractor::extract_from_ssa(&ssa)?;
/// let mut solver = AliasSolver::new(&extractor);
/// solver.solve()?;
/// let alias_info = solver.build_alias_info("my_function");
/// ```
#[derive(Debug)]
pub struct AliasSolver {
    /// Points-to sets for each variable (using formatted location strings).
    points_to: HashMap<String, HashSet<String>>,

    /// Worklist of variables with changed points-to sets.
    worklist: VecDeque<String>,

    /// Set of variables currently in the worklist (for O(1) membership check).
    in_worklist: HashSet<String>,

    /// Copy constraints: target -> sources.
    copy_constraints: HashMap<String, Vec<String>>,

    /// Reverse copy constraints: source -> targets (for propagation).
    reverse_copy: HashMap<String, Vec<String>>,

    /// Field load constraints: target -> (base, field).
    field_loads: HashMap<String, Vec<(String, String)>>,

    /// Field store constraints: base -> (field, source).
    field_stores: HashMap<String, Vec<(String, String)>>,

    /// Allocation sites: target -> AbstractLocation.
    alloc_sites: HashMap<String, AbstractLocation>,

    /// Variables that are phi function targets (may-alias only, not must-alias).
    phi_targets: HashSet<String>,

    /// Parameters (for conservative parameter aliasing).
    parameters: HashSet<String>,

    /// Variables that changed in the last iteration (for TIGER-2 debugging).
    last_changed: Vec<String>,

    /// Total number of iterations performed.
    iterations: usize,
}

impl AliasSolver {
    /// Create a new solver from extracted constraints.
    ///
    /// # Arguments
    /// * `extractor` - The constraint extractor containing constraints and metadata
    ///
    /// # Returns
    /// A new solver initialized with the constraints
    pub fn new(extractor: &ConstraintExtractor) -> Self {
        let mut solver = AliasSolver {
            points_to: HashMap::new(),
            worklist: VecDeque::new(),
            in_worklist: HashSet::new(),
            copy_constraints: HashMap::new(),
            reverse_copy: HashMap::new(),
            field_loads: HashMap::new(),
            field_stores: HashMap::new(),
            alloc_sites: HashMap::new(),
            phi_targets: extractor.phi_targets().clone(),
            parameters: extractor.parameters().clone(),
            last_changed: Vec::new(),
            iterations: 0,
        };

        // Build constraint indices for efficient lookup
        solver.index_constraints(extractor.constraints());

        // Initialize points-to sets from allocation constraints
        solver.initialize_allocs(extractor);

        // Initialize parameter points-to sets
        solver.initialize_parameters();

        solver
    }

    /// Index constraints for efficient lookup during iteration.
    fn index_constraints(&mut self, constraints: &[Constraint]) {
        for constraint in constraints {
            match constraint {
                Constraint::Copy { target, source } => {
                    // Forward: target depends on source
                    self.copy_constraints
                        .entry(target.clone())
                        .or_default()
                        .push(source.clone());
                    // Reverse: when source changes, update target
                    self.reverse_copy
                        .entry(source.clone())
                        .or_default()
                        .push(target.clone());
                }
                Constraint::Alloc { target, site } => {
                    self.alloc_sites.insert(target.clone(), site.clone());
                }
                Constraint::FieldLoad {
                    target,
                    base,
                    field,
                } => {
                    self.field_loads
                        .entry(target.clone())
                        .or_default()
                        .push((base.clone(), field.clone()));
                    // Also track reverse for propagation when base changes
                    self.reverse_copy
                        .entry(base.clone())
                        .or_default()
                        .push(target.clone());
                }
                Constraint::FieldStore {
                    base,
                    field,
                    source,
                } => {
                    self.field_stores
                        .entry(base.clone())
                        .or_default()
                        .push((field.clone(), source.clone()));
                    // Track reverse for both base and source
                    self.reverse_copy
                        .entry(source.clone())
                        .or_default()
                        .push(base.clone());
                }
            }
        }
    }

    /// Initialize points-to sets from allocation constraints.
    fn initialize_allocs(&mut self, extractor: &ConstraintExtractor) {
        for constraint in extractor.constraints() {
            if let Constraint::Alloc { target, site } = constraint {
                let location_str = site.format();
                self.points_to
                    .entry(target.clone())
                    .or_default()
                    .insert(location_str);
                self.add_to_worklist(target);
            }
        }
    }

    /// Initialize points-to sets for parameters.
    fn initialize_parameters(&mut self) {
        for param in &self.parameters.clone() {
            let location = AbstractLocation::param(param);
            let location_str = location.format();
            self.points_to
                .entry(param.clone())
                .or_default()
                .insert(location_str);
            self.add_to_worklist(param);
        }
    }

    /// Add a variable to the worklist if not already present.
    fn add_to_worklist(&mut self, var: &str) {
        if !self.in_worklist.contains(var) {
            self.worklist.push_back(var.to_string());
            self.in_worklist.insert(var.to_string());
        }
    }

    /// Run fixed-point iteration until convergence or iteration limit.
    ///
    /// # Returns
    /// * `Ok(())` if converged
    /// * `Err(AliasError::IterationLimit)` if MAX_ITERATIONS exceeded
    ///
    /// # TIGER-2 Mitigation
    /// Tracks the last-changed variables for debugging if the limit is hit.
    pub fn solve(&mut self) -> Result<(), AliasError> {
        self.iterations = 0;

        while !self.worklist.is_empty() {
            self.iterations += 1;

            if self.iterations > MAX_ITERATIONS {
                // TIGER-2: Record what was changing for debugging
                return Err(AliasError::IterationLimit(self.iterations));
            }

            // Track changes this iteration for debugging
            self.last_changed.clear();

            // Process all variables currently in the worklist
            let current_worklist: Vec<String> = self.worklist.drain(..).collect();
            self.in_worklist.clear();

            for var in current_worklist {
                self.propagate_variable(&var);
            }
        }

        Ok(())
    }

    /// Propagate points-to information for a single variable.
    fn propagate_variable(&mut self, var: &str) {
        // Get current points-to set for this variable
        let current_pts = self.points_to.get(var).cloned().unwrap_or_default();

        // Propagate to all targets that depend on this variable
        if let Some(targets) = self.reverse_copy.get(var).cloned() {
            for target in targets {
                // Check if target is a field load
                if let Some(field_loads) = self.field_loads.get(&target).cloned() {
                    for (base, field) in field_loads {
                        if base == var {
                            // Field load: pts(target) = pts(target) U {loc.field | loc in pts(base)}
                            self.propagate_field_load(&target, &current_pts, &field);
                        }
                    }
                } else if self.copy_constraints.contains_key(&target) {
                    // Copy constraint: pts(target) = pts(target) U pts(source)
                    self.propagate_copy(&target, &current_pts);
                }
            }
        }

        // Handle field stores: for base variable, update field locations
        if let Some(stores) = self.field_stores.get(var).cloned() {
            for (field, source) in stores {
                self.propagate_field_store(&current_pts, &field, &source);
            }
        }
    }

    /// Propagate copy constraint: pts(target) = pts(target) U pts(source).
    fn propagate_copy(&mut self, target: &str, source_pts: &HashSet<String>) {
        // Clone and modify to avoid borrow issues
        let mut target_pts = self.points_to.get(target).cloned().unwrap_or_default();
        let old_size = target_pts.len();

        for loc in source_pts {
            target_pts.insert(loc.clone());
        }

        let changed = target_pts.len() > old_size;
        self.points_to.insert(target.to_string(), target_pts);

        if changed {
            self.last_changed.push(target.to_string());
            self.add_to_worklist(target);
        }
    }

    /// Propagate field load: pts(target) = pts(target) U {loc.field | loc in pts(base)}.
    fn propagate_field_load(&mut self, target: &str, base_pts: &HashSet<String>, field: &str) {
        // Collect field locations first to avoid borrow issues
        let field_locs: Vec<String> = base_pts
            .iter()
            .map(|loc| self.create_field_location(loc, field))
            .collect();

        // Clone and modify to avoid borrow issues
        let mut target_pts = self.points_to.get(target).cloned().unwrap_or_default();
        let old_size = target_pts.len();

        for field_loc in field_locs {
            target_pts.insert(field_loc);
        }

        let changed = target_pts.len() > old_size;
        self.points_to.insert(target.to_string(), target_pts);

        if changed {
            self.last_changed.push(target.to_string());
            self.add_to_worklist(target);
        }
    }

    /// Propagate field store: for each loc in pts(base), pts(loc.field) U= pts(source).
    fn propagate_field_store(&mut self, base_pts: &HashSet<String>, field: &str, source: &str) {
        let source_pts = self.points_to.get(source).cloned().unwrap_or_default();

        // Collect field locations first
        let field_locs: Vec<String> = base_pts
            .iter()
            .map(|loc| self.create_field_location(loc, field))
            .collect();

        for field_loc in field_locs {
            // Clone and modify to avoid borrow issues
            let mut field_pts = self.points_to.get(&field_loc).cloned().unwrap_or_default();
            let old_size = field_pts.len();

            for source_loc in &source_pts {
                field_pts.insert(source_loc.clone());
            }

            let changed = field_pts.len() > old_size;
            self.points_to.insert(field_loc.clone(), field_pts);

            if changed {
                self.last_changed.push(field_loc.clone());
                self.add_to_worklist(&field_loc);
            }
        }
    }

    /// Create a field location string, respecting MAX_FIELD_DEPTH.
    fn create_field_location(&self, base: &str, field: &str) -> String {
        // Count existing field depth
        let depth = base.matches('.').count();

        if depth >= MAX_FIELD_DEPTH {
            // TIGER-1: Truncate deep field chains
            format!("{}.truncated", base)
        } else {
            format!("{}.{}", base, field)
        }
    }

    /// Build AliasInfo from the solved points-to sets.
    ///
    /// # Arguments
    /// * `function_name` - Name of the function being analyzed
    ///
    /// # Returns
    /// Complete alias analysis results including may-alias, must-alias,
    /// and points-to information.
    pub fn build_alias_info(&self, function_name: &str) -> AliasInfo {
        let mut info = AliasInfo::new(function_name);

        // Copy points-to sets
        info.points_to = self.points_to.clone();

        // Record allocation sites
        for (target, site) in &self.alloc_sites {
            if let AbstractLocation::Alloc { site: line } = site {
                info.add_allocation_site(*line, &site.format());
            }
            // Also add the points-to relationship
            info.add_points_to(target, &site.format());
        }

        // Compute may-alias from points-to overlap
        self.compute_may_alias(&mut info);

        // Compute must-alias from direct copies (non-phi)
        self.compute_must_alias(&mut info);

        // Add conservative parameter aliasing
        self.add_parameter_aliasing(&mut info);

        info
    }

    /// Compute may-alias relationships from points-to set overlap.
    fn compute_may_alias(&self, info: &mut AliasInfo) {
        let vars: Vec<_> = self.points_to.keys().cloned().collect();

        for i in 0..vars.len() {
            for j in (i + 1)..vars.len() {
                let v1 = &vars[i];
                let v2 = &vars[j];

                let pts1 = self.points_to.get(v1);
                let pts2 = self.points_to.get(v2);

                if let (Some(set1), Some(set2)) = (pts1, pts2) {
                    if !set1.is_disjoint(set2) {
                        info.add_may_alias(v1, v2);
                    }
                }
            }
        }

        // Add may-alias from copy constraints (transitive)
        for (target, sources) in &self.copy_constraints {
            for source in sources {
                info.add_may_alias(target, source);
            }
        }
    }

    /// Compute must-alias relationships from direct copies (non-phi).
    ///
    /// # TIGER-6 Mitigation
    /// Computes transitive closure for must-alias relationships.
    fn compute_must_alias(&self, info: &mut AliasInfo) {
        // Direct must-alias from copy constraints (exclude phi targets)
        let mut direct_aliases: HashMap<String, HashSet<String>> = HashMap::new();

        for (target, sources) in &self.copy_constraints {
            // Skip phi targets - they may-alias but don't must-alias
            if self.phi_targets.contains(target) {
                continue;
            }

            for source in sources {
                // Direct copy creates must-alias
                direct_aliases
                    .entry(target.clone())
                    .or_default()
                    .insert(source.clone());
                direct_aliases
                    .entry(source.clone())
                    .or_default()
                    .insert(target.clone());
            }
        }

        // TIGER-6: Compute transitive closure
        let transitive = self.transitive_closure(&direct_aliases);

        // Add to AliasInfo
        for (var, aliases) in transitive {
            for alias in aliases {
                info.add_must_alias(&var, &alias);
            }
        }
    }

    /// Compute transitive closure of a relation using Floyd-Warshall-style iteration.
    fn transitive_closure(
        &self,
        relation: &HashMap<String, HashSet<String>>,
    ) -> HashMap<String, HashSet<String>> {
        let mut result = relation.clone();

        // Collect all variables
        let vars: HashSet<_> = relation
            .keys()
            .chain(relation.values().flatten())
            .cloned()
            .collect();

        // Floyd-Warshall-style iteration
        let mut changed = true;
        let mut iterations = 0;

        while changed && iterations < MAX_ITERATIONS {
            changed = false;
            iterations += 1;

            // Collect all updates to apply, then apply them
            let mut updates: Vec<(String, String)> = Vec::new();

            for v in &vars {
                let current_aliases: Vec<String> = result
                    .get(v)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect();

                for alias in current_aliases {
                    // If v -> alias and alias -> x, then v -> x
                    let transitive_aliases: Vec<String> = result
                        .get(&alias)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|x| x != v)
                        .collect();

                    let v_set = result.get(v).cloned().unwrap_or_default();
                    for x in transitive_aliases {
                        if !v_set.contains(&x) {
                            updates.push((v.clone(), x.clone()));
                            updates.push((x, v.clone())); // Symmetry
                        }
                    }
                }
            }

            // Apply all updates
            for (from, to) in updates {
                if result.entry(from).or_default().insert(to) {
                    changed = true;
                }
            }
        }

        result
    }

    /// Add conservative parameter aliasing.
    ///
    /// All parameters may-alias each other because the caller could
    /// pass the same object to multiple parameters.
    fn add_parameter_aliasing(&self, info: &mut AliasInfo) {
        let params: Vec<_> = self.parameters.iter().cloned().collect();

        for i in 0..params.len() {
            for j in (i + 1)..params.len() {
                info.add_may_alias(&params[i], &params[j]);
            }
        }
    }

    /// Get the number of iterations performed.
    pub fn iterations(&self) -> usize {
        self.iterations
    }

    /// Get variables that changed in the last iteration (for debugging).
    pub fn last_changed(&self) -> &[String] {
        &self.last_changed
    }

    /// Get the current points-to set for a variable.
    pub fn get_points_to(&self, var: &str) -> HashSet<String> {
        self.points_to.get(var).cloned().unwrap_or_default()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a minimal constraint extractor for testing.
    fn create_test_extractor() -> ConstraintExtractor {
        ConstraintExtractor::new()
    }

    #[test]
    fn test_solver_new() {
        let extractor = create_test_extractor();
        let solver = AliasSolver::new(&extractor);

        assert!(solver.points_to.is_empty());
        assert!(solver.worklist.is_empty());
        assert_eq!(solver.iterations, 0);
    }

    #[test]
    fn test_solver_empty_constraints() {
        let extractor = create_test_extractor();
        let mut solver = AliasSolver::new(&extractor);

        let result = solver.solve();
        assert!(result.is_ok());
        assert_eq!(solver.iterations, 0);
    }

    #[test]
    fn test_solver_build_alias_info_empty() {
        let extractor = create_test_extractor();
        let solver = AliasSolver::new(&extractor);

        let info = solver.build_alias_info("test_func");

        assert_eq!(info.function_name, "test_func");
        assert!(info.may_alias.is_empty());
        assert!(info.must_alias.is_empty());
        assert!(info.points_to.is_empty());
    }

    #[test]
    fn test_create_field_location_simple() {
        let extractor = create_test_extractor();
        let solver = AliasSolver::new(&extractor);

        let loc = solver.create_field_location("alloc_5", "data");
        assert_eq!(loc, "alloc_5.data");
    }

    #[test]
    fn test_create_field_location_truncates_deep() {
        let extractor = create_test_extractor();
        let solver = AliasSolver::new(&extractor);

        // Create a deeply nested location string
        let mut base = "alloc_1".to_string();
        for i in 0..MAX_FIELD_DEPTH {
            base = format!("{}.field{}", base, i);
        }

        let loc = solver.create_field_location(&base, "too_deep");
        assert!(loc.ends_with(".truncated"));
    }

    #[test]
    fn test_transitive_closure_simple() {
        let extractor = create_test_extractor();
        let solver = AliasSolver::new(&extractor);

        // x -> y, y -> z => x -> z
        let mut relation: HashMap<String, HashSet<String>> = HashMap::new();
        relation.insert("x".to_string(), HashSet::from(["y".to_string()]));
        relation.insert(
            "y".to_string(),
            HashSet::from(["x".to_string(), "z".to_string()]),
        );
        relation.insert("z".to_string(), HashSet::from(["y".to_string()]));

        let result = solver.transitive_closure(&relation);

        // Check x -> z transitively
        assert!(result.get("x").is_some_and(|s| s.contains("z")));
        assert!(result.get("z").is_some_and(|s| s.contains("x")));
    }

    #[test]
    fn test_add_to_worklist_deduplication() {
        let extractor = create_test_extractor();
        let mut solver = AliasSolver::new(&extractor);

        solver.add_to_worklist("x");
        solver.add_to_worklist("x");
        solver.add_to_worklist("x");

        assert_eq!(solver.worklist.len(), 1);
    }

    #[test]
    fn test_max_iterations_constant() {
        assert_eq!(MAX_ITERATIONS, 100);
    }

    // -------------------------------------------------------------------------
    // Phase 6: Field Access Solver Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_field_location_propagation() {
        // Test that field locations are created correctly
        let extractor = create_test_extractor();
        let solver = AliasSolver::new(&extractor);

        // Simple field
        assert_eq!(
            solver.create_field_location("alloc_1", "data"),
            "alloc_1.data"
        );

        // Nested field
        assert_eq!(
            solver.create_field_location("alloc_1.inner", "value"),
            "alloc_1.inner.value"
        );

        // param field
        assert_eq!(
            solver.create_field_location("param_x", "attr"),
            "param_x.attr"
        );
    }

    #[test]
    fn test_field_depth_truncation() {
        let extractor = create_test_extractor();
        let solver = AliasSolver::new(&extractor);

        // Build a location at exactly MAX_FIELD_DEPTH
        let mut base = "alloc_1".to_string();
        for i in 0..(MAX_FIELD_DEPTH - 1) {
            base = format!("{}.f{}", base, i);
        }

        // One more field should still work
        let within_limit = solver.create_field_location(&base, "ok");
        assert!(!within_limit.contains("truncated"));

        // Adding another should trigger truncation
        let at_limit = solver.create_field_location(&within_limit, "toomuch");
        assert!(at_limit.ends_with(".truncated"));
    }

    #[test]
    fn test_solver_convergence_simple() {
        // Test that solver converges quickly for simple constraints
        let extractor = create_test_extractor();
        let mut solver = AliasSolver::new(&extractor);

        // Empty constraints should converge immediately
        let result = solver.solve();
        assert!(result.is_ok());
        assert!(solver.iterations() <= 1);
    }

    #[test]
    fn test_parameter_aliasing_in_solver() {
        use crate::ssa::types::{
            SsaBlock, SsaFunction, SsaInstruction, SsaInstructionKind, SsaName, SsaNameId,
            SsaStats, SsaType,
        };
        use std::path::PathBuf;

        // Create SSA with two parameters
        let ssa = SsaFunction {
            function: "test".to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![SsaBlock {
                id: 0,
                label: Some("entry".to_string()),
                lines: (1, 1),
                phi_functions: vec![],
                instructions: vec![
                    SsaInstruction {
                        kind: SsaInstructionKind::Param,
                        target: Some(SsaNameId(0)),
                        uses: vec![],
                        line: 1,
                        source_text: Some("def f(a, b):".to_string()),
                    },
                    SsaInstruction {
                        kind: SsaInstructionKind::Param,
                        target: Some(SsaNameId(1)),
                        uses: vec![],
                        line: 1,
                        source_text: None,
                    },
                ],
                successors: vec![],
                predecessors: vec![],
            }],
            ssa_names: vec![
                SsaName {
                    id: SsaNameId(0),
                    variable: "a".to_string(),
                    version: 0,
                    def_block: Some(0),
                    def_line: 1,
                },
                SsaName {
                    id: SsaNameId(1),
                    variable: "b".to_string(),
                    version: 0,
                    def_block: Some(0),
                    def_line: 1,
                },
            ],
            def_use: std::collections::HashMap::new(),
            stats: SsaStats::default(),
        };

        let extractor = ConstraintExtractor::extract_from_ssa(&ssa).unwrap();
        let mut solver = AliasSolver::new(&extractor);
        solver.solve().unwrap();
        let info = solver.build_alias_info("test");

        // Parameters should may-alias each other (conservative)
        assert!(info.may_alias_check("a_0", "b_0"));
    }
}
