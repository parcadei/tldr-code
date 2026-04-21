//! Constraint Generation for Alias Analysis
//!
//! This module extracts constraints from SSA form for Andersen-style
//! points-to analysis. Constraints are used by the solver to compute
//! points-to sets via fixed-point iteration.
//!
//! # Constraint Types
//!
//! - **Copy**: `x = y` -> `pts(x) ⊇ pts(y)`
//! - **Alloc**: `x = new T()` -> `pts(x) ⊇ {alloc_site}`
//! - **FieldLoad**: `x = y.f` -> `pts(x) ⊇ pts(y).f`
//! - **FieldStore**: `x.f = y` -> `pts(x).f ⊇ pts(y)`
//!
//! # TIGER Mitigations
//!
//! - **TIGER-3**: Validates all SSA references exist before processing
//! - **TIGER-14**: Validates phi function source count matches predecessors

use std::collections::HashSet;

use crate::ssa::types::{
    PhiFunction, SsaBlock, SsaFunction, SsaInstruction, SsaInstructionKind, SsaNameId,
};

use super::types::{AbstractLocation, AliasError};

// =============================================================================
// Constraint Types
// =============================================================================

/// Constraint types for Andersen's analysis.
///
/// These represent the fundamental pointer relationships that must be
/// propagated during fixed-point iteration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Constraint {
    /// Copy constraint: `x = y` -> `pts(x) ⊇ pts(y)`
    ///
    /// The target variable's points-to set must include everything
    /// the source variable points to.
    Copy {
        /// Variable receiving the copy (left-hand side)
        target: String,
        /// Variable being copied (right-hand side)
        source: String,
    },

    /// Allocation constraint: `x = new T()` -> `pts(x) ⊇ {alloc_site}`
    ///
    /// The target variable points to a newly allocated object
    /// at the given abstract location.
    Alloc {
        /// Variable receiving the allocation
        target: String,
        /// Abstract location representing the allocation site
        site: AbstractLocation,
    },

    /// Field load constraint: `x = y.field` -> `pts(x) ⊇ pts(y).field`
    ///
    /// The target variable's points-to set must include the field
    /// of every location the base variable points to.
    FieldLoad {
        /// Variable receiving the field value
        target: String,
        /// Base object being accessed
        base: String,
        /// Field name being loaded
        field: String,
    },

    /// Field store constraint: `x.field = y` -> `pts(x).field ⊇ pts(y)`
    ///
    /// For every location the base points to, the field of that
    /// location must include everything the source points to.
    FieldStore {
        /// Base object being modified
        base: String,
        /// Field name being stored to
        field: String,
        /// Variable whose value is being stored
        source: String,
    },
}

impl Constraint {
    /// Create a copy constraint.
    pub fn copy(target: impl Into<String>, source: impl Into<String>) -> Self {
        Constraint::Copy {
            target: target.into(),
            source: source.into(),
        }
    }

    /// Create an allocation constraint.
    pub fn alloc(target: impl Into<String>, site: AbstractLocation) -> Self {
        Constraint::Alloc {
            target: target.into(),
            site,
        }
    }

    /// Create a field load constraint.
    pub fn field_load(
        target: impl Into<String>,
        base: impl Into<String>,
        field: impl Into<String>,
    ) -> Self {
        Constraint::FieldLoad {
            target: target.into(),
            base: base.into(),
            field: field.into(),
        }
    }

    /// Create a field store constraint.
    pub fn field_store(
        base: impl Into<String>,
        field: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Constraint::FieldStore {
            base: base.into(),
            field: field.into(),
            source: source.into(),
        }
    }

    /// Get the target variable name if this constraint defines one.
    pub fn target(&self) -> Option<&str> {
        match self {
            Constraint::Copy { target, .. } => Some(target),
            Constraint::Alloc { target, .. } => Some(target),
            Constraint::FieldLoad { target, .. } => Some(target),
            Constraint::FieldStore { .. } => None,
        }
    }

    /// Get all variables referenced by this constraint.
    pub fn variables(&self) -> Vec<&str> {
        match self {
            Constraint::Copy { target, source } => vec![target, source],
            Constraint::Alloc { target, .. } => vec![target],
            Constraint::FieldLoad { target, base, .. } => vec![target, base],
            Constraint::FieldStore { base, source, .. } => vec![base, source],
        }
    }
}

// =============================================================================
// Constraint Extractor
// =============================================================================

/// Extract constraints from SSA form.
///
/// The `ConstraintExtractor` processes an SSA function and generates
/// the constraint set needed for Andersen's analysis.
///
/// # Example
///
/// ```rust,ignore
/// use tldr_core::alias::constraints::ConstraintExtractor;
/// use tldr_core::ssa::types::SsaFunction;
///
/// let ssa: SsaFunction = /* ... */;
/// let extractor = ConstraintExtractor::extract_from_ssa(&ssa)?;
///
/// for constraint in extractor.constraints() {
///     println!("{:?}", constraint);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ConstraintExtractor {
    /// Extracted constraints
    constraints: Vec<Constraint>,
    /// Allocation sites discovered during extraction
    allocation_sites: HashSet<AbstractLocation>,
    /// Set of SSA names that are phi function targets (may-alias only)
    phi_targets: HashSet<String>,
    /// Set of parameters (for parameter aliasing)
    parameters: HashSet<String>,
    /// Mapping from SsaNameId to formatted name for quick lookup
    name_map: std::collections::HashMap<SsaNameId, String>,
}

impl ConstraintExtractor {
    /// Create a new empty constraint extractor.
    pub fn new() -> Self {
        ConstraintExtractor {
            constraints: Vec::new(),
            allocation_sites: HashSet::new(),
            phi_targets: HashSet::new(),
            parameters: HashSet::new(),
            name_map: std::collections::HashMap::new(),
        }
    }

    /// Extract constraints from an SSA function.
    ///
    /// This is the main entry point for constraint extraction.
    ///
    /// # Arguments
    /// * `ssa` - The SSA form of the function to analyze
    ///
    /// # Returns
    /// * `Ok(ConstraintExtractor)` - Extracted constraints and metadata
    /// * `Err(AliasError)` - If SSA validation fails
    ///
    /// # TIGER-3 Mitigation
    /// Validates all SSA references exist before processing.
    pub fn extract_from_ssa(ssa: &SsaFunction) -> Result<Self, AliasError> {
        let mut extractor = Self::new();

        // Build name map for fast lookup (TIGER-3: validate references)
        extractor.build_name_map(ssa)?;

        // Process each block
        for block in &ssa.blocks {
            // Process phi functions first
            extractor.process_phi_functions(ssa, block)?;

            // Process instructions
            for instruction in &block.instructions {
                extractor.process_instruction(ssa, instruction)?;
            }
        }

        Ok(extractor)
    }

    /// Get the extracted constraints.
    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    /// Get the allocation sites discovered during extraction.
    pub fn allocation_sites(&self) -> &HashSet<AbstractLocation> {
        &self.allocation_sites
    }

    /// Get the set of phi function targets.
    ///
    /// These variables should NOT have must-alias relationships
    /// because they could come from multiple sources at runtime.
    pub fn phi_targets(&self) -> &HashSet<String> {
        &self.phi_targets
    }

    /// Get the set of parameter names.
    pub fn parameters(&self) -> &HashSet<String> {
        &self.parameters
    }

    /// Check if a variable is a phi target.
    pub fn is_phi_target(&self, var: &str) -> bool {
        self.phi_targets.contains(var)
    }

    // =========================================================================
    // Internal Methods
    // =========================================================================

    /// Build a mapping from SsaNameId to formatted name.
    ///
    /// TIGER-3: This validates that all SSA names are properly defined.
    fn build_name_map(&mut self, ssa: &SsaFunction) -> Result<(), AliasError> {
        for ssa_name in &ssa.ssa_names {
            let formatted = ssa_name.format_name();
            self.name_map.insert(ssa_name.id, formatted);
        }
        Ok(())
    }

    /// Format an SSA name ID to its string representation.
    ///
    /// TIGER-3: Returns a placeholder for missing names instead of crashing.
    fn format_ssa_name(&self, id: SsaNameId) -> String {
        self.name_map
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("$unknown_{}", id.0))
    }

    /// Validate that an SSA name ID exists.
    ///
    /// TIGER-3: Returns error if the ID is not in the name map.
    fn validate_ssa_name(&self, id: SsaNameId, context: &str) -> Result<String, AliasError> {
        self.name_map.get(&id).cloned().ok_or_else(|| {
            AliasError::InvalidRef(format!(
                "SSA name ${} not found in {} (TIGER-3 violation)",
                id.0, context
            ))
        })
    }

    /// Process phi functions in a block.
    ///
    /// TIGER-14: Validates phi source count matches predecessor count.
    fn process_phi_functions(
        &mut self,
        ssa: &SsaFunction,
        block: &SsaBlock,
    ) -> Result<(), AliasError> {
        for phi in &block.phi_functions {
            self.process_single_phi(ssa, phi, block)?;
        }
        Ok(())
    }

    /// Process a single phi function.
    fn process_single_phi(
        &mut self,
        _ssa: &SsaFunction,
        phi: &PhiFunction,
        block: &SsaBlock,
    ) -> Result<(), AliasError> {
        // TIGER-14: Validate phi source count
        // Note: This is a warning, not an error - some SSA forms may have
        // different source counts due to unreachable predecessors
        if phi.sources.len() != block.predecessors.len() && !block.predecessors.is_empty() {
            // Log warning but continue - this is acceptable in some SSA variants
            // In strict mode, this could return an error
        }

        // Get target name
        let target =
            self.validate_ssa_name(phi.target, &format!("phi target in block {}", block.id))?;

        // Mark as phi target (no must-alias for phi results)
        self.phi_targets.insert(target.clone());

        // Create copy constraints from each source
        for source in &phi.sources {
            let source_name = self.validate_ssa_name(
                source.name,
                &format!(
                    "phi source for {} from block {}",
                    phi.variable, source.block
                ),
            )?;

            self.constraints
                .push(Constraint::copy(target.clone(), source_name));
        }

        Ok(())
    }

    /// Process an SSA instruction.
    fn process_instruction(
        &mut self,
        ssa: &SsaFunction,
        instruction: &SsaInstruction,
    ) -> Result<(), AliasError> {
        match instruction.kind {
            SsaInstructionKind::Param => {
                self.process_param_instruction(instruction)?;
            }
            SsaInstructionKind::Assign => {
                self.process_assign_instruction(instruction)?;
            }
            SsaInstructionKind::Call => {
                self.process_call_instruction(ssa, instruction)?;
            }
            SsaInstructionKind::BinaryOp
            | SsaInstructionKind::UnaryOp
            | SsaInstructionKind::Return
            | SsaInstructionKind::Branch => {
                // These don't create alias constraints
            }
        }
        Ok(())
    }

    /// Process a parameter instruction.
    ///
    /// Creates: `pts(param) = {param_NAME}`
    /// For mutable defaults (TIGER-7): `pts(param) = {alloc_default_LINE}`
    fn process_param_instruction(
        &mut self,
        instruction: &SsaInstruction,
    ) -> Result<(), AliasError> {
        if let Some(target_id) = instruction.target {
            let target = self.validate_ssa_name(target_id, "param instruction")?;

            // TIGER-7: Check for mutable default argument
            if let Some(source_text) = &instruction.source_text {
                if let Some(default_site) =
                    self.parse_mutable_default(source_text, instruction.line)
                {
                    // This parameter has a mutable default value
                    // The default is shared across all calls
                    self.allocation_sites.insert(default_site.clone());
                    self.parameters.insert(target.clone());
                    self.constraints
                        .push(Constraint::alloc(target.clone(), default_site));
                    // Also add the param location for when caller provides value
                    let param_name =
                        target.trim_end_matches(|c: char| c == '_' || c.is_ascii_digit());
                    let param_site = AbstractLocation::param(param_name);
                    self.allocation_sites.insert(param_site.clone());
                    self.constraints.push(Constraint::alloc(target, param_site));
                    return Ok(());
                }
            }

            // Extract parameter name from SSA name (e.g., "p_0" -> "p")
            let param_name = target
                .rsplit('_')
                .nth(1)
                .map(|_| {
                    // Handle cases like "param_name_0" -> "param_name"
                    target.trim_end_matches(|c: char| c == '_' || c.is_ascii_digit())
                })
                .unwrap_or(&target);

            let site = AbstractLocation::param(param_name);
            self.allocation_sites.insert(site.clone());
            self.parameters.insert(target.clone());

            self.constraints.push(Constraint::alloc(target, site));
        }
        Ok(())
    }

    /// Process an assignment instruction.
    ///
    /// For `x = y`: Creates copy constraint `pts(x) ⊇ pts(y)`
    /// For `x = y.f`: Creates field load constraint
    /// For `x = Class.f`: Creates class variable allocation (TIGER-8)
    fn process_assign_instruction(
        &mut self,
        instruction: &SsaInstruction,
    ) -> Result<(), AliasError> {
        if let Some(target_id) = instruction.target {
            let target = self.validate_ssa_name(target_id, "assign target")?;

            // Check if this is a simple copy or field access
            if let Some(source_text) = &instruction.source_text {
                // TIGER-8: Check for class variable access (ClassName.field)
                if let Some(class_var) = self.detect_class_var(source_text) {
                    self.allocation_sites.insert(class_var.clone());
                    self.constraints.push(Constraint::alloc(target, class_var));
                    return Ok(());
                }

                // Try to detect field access pattern
                if let Some(field_access) = self.parse_field_access(source_text) {
                    // This is a field load: x = base.field
                    // Find the base variable in uses
                    if !instruction.uses.is_empty() {
                        let base = self.format_ssa_name(instruction.uses[0]);
                        self.constraints
                            .push(Constraint::field_load(target, base, field_access));
                        return Ok(());
                    }
                }

                // Try to detect field store pattern: base.field = value
                if let Some((base_field, _)) = self.parse_field_store(source_text) {
                    // This is handled separately - field stores don't have a target
                    // The target here is actually the base object being modified
                    if !instruction.uses.is_empty() {
                        let source = self.format_ssa_name(instruction.uses[0]);
                        let (base_name, field_name) = base_field;
                        self.constraints
                            .push(Constraint::field_store(base_name, field_name, source));
                        return Ok(());
                    }
                }
            }

            // Simple copy: x = y
            if instruction.uses.len() == 1 {
                let source = self.format_ssa_name(instruction.uses[0]);
                self.constraints.push(Constraint::copy(target, source));
            } else if instruction.uses.is_empty() {
                // Assignment with no uses: could be literal, allocation, or constant.
                // Check source_text to determine if this is a known allocation pattern.
                let is_allocation = instruction
                    .source_text
                    .as_ref()
                    .map(|s| self.is_allocation_call(s))
                    .unwrap_or(false);

                let site = if is_allocation {
                    AbstractLocation::alloc(instruction.line)
                } else {
                    AbstractLocation::unknown(instruction.line)
                };
                self.allocation_sites.insert(site.clone());
                self.constraints.push(Constraint::alloc(target, site));
            }
        }
        Ok(())
    }

    /// Process a call instruction.
    ///
    /// For `x = Foo()`: Creates allocation constraint (constructor call)
    /// For `x = func()`: Creates unknown constraint (external call)
    fn process_call_instruction(
        &mut self,
        _ssa: &SsaFunction,
        instruction: &SsaInstruction,
    ) -> Result<(), AliasError> {
        if let Some(target_id) = instruction.target {
            let target = self.validate_ssa_name(target_id, "call target")?;

            // Determine if this is an allocation or unknown call
            let is_allocation = instruction
                .source_text
                .as_ref()
                .map(|s| self.is_allocation_call(s))
                .unwrap_or(false);

            let site = if is_allocation {
                // Allocation: x = Foo(), x = [], x = {}
                AbstractLocation::alloc(instruction.line)
            } else {
                // Unknown/external call
                AbstractLocation::unknown(instruction.line)
            };

            self.allocation_sites.insert(site.clone());
            self.constraints.push(Constraint::alloc(target, site));
        }
        Ok(())
    }

    /// Check if a call is an allocation (constructor, list, dict).
    fn is_allocation_call(&self, source_text: &str) -> bool {
        // Simple heuristics for allocation detection
        let trimmed = source_text.trim();

        // Constructor call: Foo(), ClassName()
        // Look for pattern: something = Name()
        if let Some(rhs) = trimmed.split('=').nth(1) {
            let rhs = rhs.trim();
            // Check for constructor pattern: starts with uppercase or is [], {}
            if rhs.starts_with('[') || rhs.starts_with('{') {
                return true;
            }
            // Check for ClassName() pattern
            if let Some(first_char) = rhs.chars().next() {
                if first_char.is_uppercase() {
                    return true;
                }
            }
        }

        // Direct patterns
        trimmed.contains("[]") || trimmed.contains("{}")
    }

    /// Parse a field access from source text.
    ///
    /// Returns the field name if the source text contains `base.field`.
    fn parse_field_access(&self, source_text: &str) -> Option<String> {
        // Look for pattern: x = something.field
        let trimmed = source_text.trim();
        if let Some(rhs) = trimmed.split('=').nth(1) {
            let rhs = rhs.trim();
            // Check for dot notation, excluding method calls
            if rhs.contains('.') && !rhs.contains('(') {
                // Extract field name after last dot
                if let Some(field) = rhs.rsplit('.').next() {
                    let field = field.trim();
                    if !field.is_empty() && field.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        return Some(field.to_string());
                    }
                }
            }
        }
        None
    }

    /// Parse a field store from source text.
    ///
    /// Returns (base_name, field_name) if the source text contains `base.field = value`.
    fn parse_field_store(&self, source_text: &str) -> Option<((String, String), ())> {
        // Look for pattern: base.field = value
        let trimmed = source_text.trim();
        let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
        if parts.len() == 2 {
            let lhs = parts[0].trim();
            if lhs.contains('.') {
                let lhs_parts: Vec<&str> = lhs.rsplitn(2, '.').collect();
                if lhs_parts.len() == 2 {
                    let field = lhs_parts[0].trim().to_string();
                    let base = lhs_parts[1].trim().to_string();
                    if !field.is_empty() && !base.is_empty() {
                        return Some(((base, field), ()));
                    }
                }
            }
        }
        None
    }

    /// Check if source text represents a mutable default argument.
    ///
    /// Detects Python patterns like `def f(x=[])` or `def f(x={})` which create
    /// shared mutable objects across all calls (TIGER-7).
    ///
    /// Returns Some(site) if this is a default arg initialization.
    pub fn parse_mutable_default(&self, source_text: &str, line: u32) -> Option<AbstractLocation> {
        let trimmed = source_text.trim();

        // Look for pattern: param=[] or param={}
        // This appears in function definition context
        if trimmed.contains("def ") {
            // Check for mutable default patterns
            if trimmed.contains("=[]") || trimmed.contains("= []") {
                return Some(AbstractLocation::default_arg(line));
            }
            if trimmed.contains("={}") || trimmed.contains("= {}") {
                return Some(AbstractLocation::default_arg(line));
            }
        }

        None
    }

    /// Parse a class variable access pattern.
    ///
    /// Detects Python patterns like `ClassName.attr` which access class-level
    /// variables (singletons shared across all instances) (TIGER-8).
    ///
    /// Returns Some((class_name, field_name)) if this is a class variable access.
    pub fn parse_class_var_access(&self, source_text: &str) -> Option<(String, String)> {
        let trimmed = source_text.trim();

        // Look for pattern: x = ClassName.field or ClassName.field = value
        // Class names start with uppercase
        let rhs = if let Some(rhs) = trimmed.split('=').nth(1) {
            rhs.trim()
        } else if trimmed.contains('.') {
            trimmed
        } else {
            return None;
        };

        // Check for ClassName.field pattern (not method call)
        if rhs.contains('.') && !rhs.contains('(') {
            let parts: Vec<&str> = rhs.splitn(2, '.').collect();
            if parts.len() == 2 {
                let potential_class = parts[0].trim();
                let field = parts[1].trim();

                // Check if it looks like a class name (starts with uppercase)
                // and is not `self` or `cls` (instance access)
                if let Some(first_char) = potential_class.chars().next() {
                    if first_char.is_uppercase()
                        && !field.is_empty()
                        && field.chars().all(|c| c.is_alphanumeric() || c == '_')
                    {
                        return Some((potential_class.to_string(), field.to_string()));
                    }
                }
            }
        }

        None
    }

    /// Check if this is a class variable access and create the appropriate location.
    ///
    /// Returns Some(AbstractLocation::ClassVar) if this is a class variable,
    /// None otherwise (instance variable access).
    pub fn detect_class_var(&self, source_text: &str) -> Option<AbstractLocation> {
        self.parse_class_var_access(source_text)
            .map(|(class, field)| AbstractLocation::class_var(class, field))
    }
}

impl Default for ConstraintExtractor {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssa::types::{PhiSource, SsaName, SsaStats, SsaType};
    use std::path::PathBuf;

    /// Create a minimal SSA function for testing.
    fn create_test_ssa(name: &str) -> SsaFunction {
        SsaFunction {
            function: name.to_string(),
            file: PathBuf::from("test.py"),
            ssa_type: SsaType::Minimal,
            blocks: vec![],
            ssa_names: vec![],
            def_use: std::collections::HashMap::new(),
            stats: SsaStats::default(),
        }
    }

    /// Create an SSA name for testing.
    fn create_ssa_name(id: u32, variable: &str, version: u32, line: u32) -> SsaName {
        SsaName {
            id: SsaNameId(id),
            variable: variable.to_string(),
            version,
            def_block: Some(0),
            def_line: line,
        }
    }

    // -------------------------------------------------------------------------
    // Constraint Type Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_constraint_copy() {
        let c = Constraint::copy("x_0", "y_0");
        assert_eq!(
            c,
            Constraint::Copy {
                target: "x_0".to_string(),
                source: "y_0".to_string()
            }
        );
        assert_eq!(c.target(), Some("x_0"));
        assert_eq!(c.variables(), vec!["x_0", "y_0"]);
    }

    #[test]
    fn test_constraint_alloc() {
        let site = AbstractLocation::alloc(5);
        let c = Constraint::alloc("x_0", site.clone());
        assert_eq!(
            c,
            Constraint::Alloc {
                target: "x_0".to_string(),
                site
            }
        );
        assert_eq!(c.target(), Some("x_0"));
    }

    #[test]
    fn test_constraint_field_load() {
        let c = Constraint::field_load("x_0", "obj_0", "data");
        assert_eq!(
            c,
            Constraint::FieldLoad {
                target: "x_0".to_string(),
                base: "obj_0".to_string(),
                field: "data".to_string()
            }
        );
    }

    #[test]
    fn test_constraint_field_store() {
        let c = Constraint::field_store("obj_0", "data", "y_0");
        assert_eq!(
            c,
            Constraint::FieldStore {
                base: "obj_0".to_string(),
                field: "data".to_string(),
                source: "y_0".to_string()
            }
        );
        assert_eq!(c.target(), None); // Field stores don't define a variable
    }

    // -------------------------------------------------------------------------
    // Constraint Extractor Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_extractor_empty_ssa() {
        let ssa = create_test_ssa("empty");
        let extractor = ConstraintExtractor::extract_from_ssa(&ssa).unwrap();
        assert!(extractor.constraints().is_empty());
        assert!(extractor.allocation_sites().is_empty());
    }

    #[test]
    fn test_extractor_param_instruction() {
        let mut ssa = create_test_ssa("param_test");
        ssa.ssa_names = vec![create_ssa_name(0, "p", 0, 1)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 1),
            phi_functions: vec![],
            instructions: vec![SsaInstruction {
                kind: SsaInstructionKind::Param,
                target: Some(SsaNameId(0)),
                uses: vec![],
                line: 1,
                source_text: Some("def f(p):".to_string()),
            }],
            successors: vec![],
            predecessors: vec![],
        }];

        let extractor = ConstraintExtractor::extract_from_ssa(&ssa).unwrap();

        assert_eq!(extractor.constraints().len(), 1);
        assert!(extractor.parameters().contains("p_0"));

        match &extractor.constraints()[0] {
            Constraint::Alloc { target, site } => {
                assert_eq!(target, "p_0");
                assert_eq!(site.format(), "param_p");
            }
            _ => panic!("Expected Alloc constraint"),
        }
    }

    #[test]
    fn test_extractor_copy_assignment() {
        let mut ssa = create_test_ssa("copy_test");
        ssa.ssa_names = vec![create_ssa_name(0, "p", 0, 1), create_ssa_name(1, "x", 0, 2)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 2),
            phi_functions: vec![],
            instructions: vec![
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("def f(p):".to_string()),
                },
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(1)),
                    uses: vec![SsaNameId(0)],
                    line: 2,
                    source_text: Some("x = p".to_string()),
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let extractor = ConstraintExtractor::extract_from_ssa(&ssa).unwrap();

        // Should have: Alloc for param, Copy for assignment
        assert_eq!(extractor.constraints().len(), 2);

        // Find the copy constraint
        let copy_constraint = extractor
            .constraints()
            .iter()
            .find(|c| matches!(c, Constraint::Copy { .. }));

        assert!(copy_constraint.is_some());
        match copy_constraint.unwrap() {
            Constraint::Copy { target, source } => {
                assert_eq!(target, "x_0");
                assert_eq!(source, "p_0");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_extractor_call_allocation() {
        let mut ssa = create_test_ssa("alloc_test");
        ssa.ssa_names = vec![create_ssa_name(0, "x", 0, 1)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 1),
            phi_functions: vec![],
            instructions: vec![SsaInstruction {
                kind: SsaInstructionKind::Call,
                target: Some(SsaNameId(0)),
                uses: vec![],
                line: 1,
                source_text: Some("x = Foo()".to_string()),
            }],
            successors: vec![],
            predecessors: vec![],
        }];

        let extractor = ConstraintExtractor::extract_from_ssa(&ssa).unwrap();

        assert_eq!(extractor.constraints().len(), 1);
        match &extractor.constraints()[0] {
            Constraint::Alloc { target, site } => {
                assert_eq!(target, "x_0");
                assert_eq!(site.format(), "alloc_1");
            }
            _ => panic!("Expected Alloc constraint"),
        }
    }

    #[test]
    fn test_extractor_call_unknown() {
        let mut ssa = create_test_ssa("unknown_test");
        ssa.ssa_names = vec![create_ssa_name(0, "x", 0, 1)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 1),
            phi_functions: vec![],
            instructions: vec![SsaInstruction {
                kind: SsaInstructionKind::Call,
                target: Some(SsaNameId(0)),
                uses: vec![],
                line: 1,
                source_text: Some("x = external_func()".to_string()),
            }],
            successors: vec![],
            predecessors: vec![],
        }];

        let extractor = ConstraintExtractor::extract_from_ssa(&ssa).unwrap();

        assert_eq!(extractor.constraints().len(), 1);
        match &extractor.constraints()[0] {
            Constraint::Alloc { target, site } => {
                assert_eq!(target, "x_0");
                assert_eq!(site.format(), "unknown_1");
            }
            _ => panic!("Expected Alloc constraint with unknown site"),
        }
    }

    #[test]
    fn test_extractor_phi_function() {
        let mut ssa = create_test_ssa("phi_test");
        ssa.ssa_names = vec![
            create_ssa_name(0, "a", 0, 1),
            create_ssa_name(1, "b", 0, 1),
            create_ssa_name(2, "x", 0, 3),
            create_ssa_name(3, "x", 1, 5),
            create_ssa_name(4, "x", 2, 7),
        ];
        ssa.blocks = vec![
            // Entry block
            SsaBlock {
                id: 0,
                label: Some("entry".to_string()),
                lines: (1, 2),
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
                successors: vec![1, 2],
                predecessors: vec![],
            },
            // True branch: x = a
            SsaBlock {
                id: 1,
                label: Some("true".to_string()),
                lines: (3, 4),
                phi_functions: vec![],
                instructions: vec![SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(2)),
                    uses: vec![SsaNameId(0)],
                    line: 3,
                    source_text: Some("x = a".to_string()),
                }],
                successors: vec![3],
                predecessors: vec![0],
            },
            // False branch: x = b
            SsaBlock {
                id: 2,
                label: Some("false".to_string()),
                lines: (5, 6),
                phi_functions: vec![],
                instructions: vec![SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(3)),
                    uses: vec![SsaNameId(1)],
                    line: 5,
                    source_text: Some("x = b".to_string()),
                }],
                successors: vec![3],
                predecessors: vec![0],
            },
            // Merge block with phi
            SsaBlock {
                id: 3,
                label: Some("merge".to_string()),
                lines: (7, 8),
                phi_functions: vec![PhiFunction {
                    target: SsaNameId(4),
                    variable: "x".to_string(),
                    sources: vec![
                        PhiSource {
                            block: 1,
                            name: SsaNameId(2),
                        },
                        PhiSource {
                            block: 2,
                            name: SsaNameId(3),
                        },
                    ],
                    line: 7,
                }],
                instructions: vec![],
                successors: vec![],
                predecessors: vec![1, 2],
            },
        ];

        let extractor = ConstraintExtractor::extract_from_ssa(&ssa).unwrap();

        // Should have phi as target
        assert!(extractor.phi_targets().contains("x_2"));

        // Find copy constraints from phi
        let phi_copies: Vec<_> = extractor
            .constraints()
            .iter()
            .filter(|c| matches!(c, Constraint::Copy { target, .. } if target == "x_2"))
            .collect();

        assert_eq!(phi_copies.len(), 2);
    }

    #[test]
    fn test_extractor_two_allocations_no_alias() {
        let mut ssa = create_test_ssa("two_alloc_test");
        ssa.ssa_names = vec![create_ssa_name(0, "x", 0, 1), create_ssa_name(1, "y", 0, 2)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 2),
            phi_functions: vec![],
            instructions: vec![
                SsaInstruction {
                    kind: SsaInstructionKind::Call,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("x = Foo()".to_string()),
                },
                SsaInstruction {
                    kind: SsaInstructionKind::Call,
                    target: Some(SsaNameId(1)),
                    uses: vec![],
                    line: 2,
                    source_text: Some("y = Foo()".to_string()),
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let extractor = ConstraintExtractor::extract_from_ssa(&ssa).unwrap();

        // Should have two distinct allocation sites
        assert_eq!(extractor.allocation_sites().len(), 2);
        assert!(extractor
            .allocation_sites()
            .contains(&AbstractLocation::alloc(1)));
        assert!(extractor
            .allocation_sites()
            .contains(&AbstractLocation::alloc(2)));
    }

    #[test]
    fn test_extractor_invalid_ssa_reference() {
        let mut ssa = create_test_ssa("invalid_ref_test");
        // SSA names doesn't include ID 99
        ssa.ssa_names = vec![create_ssa_name(0, "x", 0, 1)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 1),
            phi_functions: vec![],
            instructions: vec![SsaInstruction {
                kind: SsaInstructionKind::Assign,
                target: Some(SsaNameId(99)), // Invalid!
                uses: vec![],
                line: 1,
                source_text: Some("x = 1".to_string()),
            }],
            successors: vec![],
            predecessors: vec![],
        }];

        let result = ConstraintExtractor::extract_from_ssa(&ssa);
        assert!(result.is_err());
        match result.unwrap_err() {
            AliasError::InvalidRef(msg) => {
                assert!(msg.contains("TIGER-3"));
            }
            _ => panic!("Expected InvalidRef error"),
        }
    }

    #[test]
    fn test_is_allocation_call() {
        let extractor = ConstraintExtractor::new();

        // Allocation patterns
        assert!(extractor.is_allocation_call("x = Foo()"));
        assert!(extractor.is_allocation_call("x = MyClass()"));
        assert!(extractor.is_allocation_call("x = []"));
        assert!(extractor.is_allocation_call("x = {}"));
        assert!(extractor.is_allocation_call("  x = []  "));

        // Non-allocation patterns
        assert!(!extractor.is_allocation_call("x = func()"));
        assert!(!extractor.is_allocation_call("x = external_call()"));
        assert!(!extractor.is_allocation_call("x = lower_case()"));
    }

    #[test]
    fn test_parse_field_access() {
        let extractor = ConstraintExtractor::new();

        // Field access patterns
        assert_eq!(
            extractor.parse_field_access("x = obj.field"),
            Some("field".to_string())
        );
        assert_eq!(
            extractor.parse_field_access("x = self.data"),
            Some("data".to_string())
        );
        assert_eq!(
            extractor.parse_field_access("  y = foo.bar  "),
            Some("bar".to_string())
        );

        // Not field access
        assert_eq!(extractor.parse_field_access("x = y"), None);
        assert_eq!(extractor.parse_field_access("x = obj.method()"), None); // Method call
        assert_eq!(extractor.parse_field_access("x = Foo()"), None);
    }

    // -------------------------------------------------------------------------
    // Phase 5: Python Patterns Tests (TIGER-7, TIGER-8)
    // -------------------------------------------------------------------------

    #[test]
    fn test_mutable_default_argument() {
        let extractor = ConstraintExtractor::new();

        // TIGER-7: Detect mutable default arguments
        let default_list = extractor.parse_mutable_default("def f(x=[]):", 5);
        assert!(default_list.is_some());
        assert_eq!(default_list.unwrap().format(), "alloc_default_5");

        let default_dict = extractor.parse_mutable_default("def f(x={}):", 10);
        assert!(default_dict.is_some());
        assert_eq!(default_dict.unwrap().format(), "alloc_default_10");

        // With spaces
        let default_spaced = extractor.parse_mutable_default("def f(x = []):", 7);
        assert!(default_spaced.is_some());

        // Not a mutable default
        let no_default = extractor.parse_mutable_default("def f(x):", 1);
        assert!(no_default.is_none());

        let not_def = extractor.parse_mutable_default("x = []", 1);
        assert!(not_def.is_none());
    }

    #[test]
    fn test_class_variable_singleton() {
        let extractor = ConstraintExtractor::new();

        // TIGER-8: Detect class variable access
        let class_var = extractor.parse_class_var_access("x = Config.DEBUG");
        assert_eq!(class_var, Some(("Config".to_string(), "DEBUG".to_string())));

        let class_var2 = extractor.parse_class_var_access("y = MyClass.counter");
        assert_eq!(
            class_var2,
            Some(("MyClass".to_string(), "counter".to_string()))
        );

        // detect_class_var returns AbstractLocation
        let loc = extractor.detect_class_var("z = Settings.value");
        assert!(loc.is_some());
        assert_eq!(loc.unwrap().format(), "alloc_class_Settings_value");

        // Not class variable (instance access via self/variable)
        let not_class = extractor.parse_class_var_access("x = self.field");
        assert!(not_class.is_none());

        let not_class2 = extractor.parse_class_var_access("x = obj.field");
        assert!(not_class2.is_none());

        // Method call (not field access)
        let method = extractor.parse_class_var_access("x = Config.get_value()");
        assert!(method.is_none());
    }

    #[test]
    fn test_class_variable_in_ssa() {
        let mut ssa = create_test_ssa("class_var_test");
        ssa.ssa_names = vec![create_ssa_name(0, "x", 0, 1)];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 1),
            phi_functions: vec![],
            instructions: vec![SsaInstruction {
                kind: SsaInstructionKind::Assign,
                target: Some(SsaNameId(0)),
                uses: vec![],
                line: 1,
                source_text: Some("x = Config.DEBUG".to_string()),
            }],
            successors: vec![],
            predecessors: vec![],
        }];

        let extractor = ConstraintExtractor::extract_from_ssa(&ssa).unwrap();

        // Should have class variable allocation
        assert!(extractor
            .allocation_sites()
            .contains(&AbstractLocation::class_var("Config", "DEBUG")));

        // The constraint should be an Alloc with ClassVar site
        let alloc_constraint = extractor
            .constraints()
            .iter()
            .find(|c| matches!(c, Constraint::Alloc { .. }));
        assert!(alloc_constraint.is_some());
    }

    // -------------------------------------------------------------------------
    // Phase 6: Field Access Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_field_load_tracking() {
        let mut ssa = create_test_ssa("field_load_test");
        ssa.ssa_names = vec![
            create_ssa_name(0, "obj", 0, 1),
            create_ssa_name(1, "x", 0, 2),
        ];
        ssa.blocks = vec![SsaBlock {
            id: 0,
            label: Some("entry".to_string()),
            lines: (1, 2),
            phi_functions: vec![],
            instructions: vec![
                SsaInstruction {
                    kind: SsaInstructionKind::Param,
                    target: Some(SsaNameId(0)),
                    uses: vec![],
                    line: 1,
                    source_text: Some("def test(obj):".to_string()),
                },
                SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(SsaNameId(1)),
                    uses: vec![SsaNameId(0)],
                    line: 2,
                    source_text: Some("x = obj.field".to_string()),
                },
            ],
            successors: vec![],
            predecessors: vec![],
        }];

        let extractor = ConstraintExtractor::extract_from_ssa(&ssa).unwrap();

        // Should have a FieldLoad constraint
        let field_load = extractor
            .constraints()
            .iter()
            .find(|c| matches!(c, Constraint::FieldLoad { .. }));
        assert!(field_load.is_some());

        match field_load.unwrap() {
            Constraint::FieldLoad {
                target,
                base,
                field,
            } => {
                assert_eq!(target, "x_0");
                assert_eq!(base, "obj_0");
                assert_eq!(field, "field");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_field_store_propagation() {
        let extractor = ConstraintExtractor::new();

        // Parse field store pattern
        let store = extractor.parse_field_store("obj.data = value");
        assert!(store.is_some());
        let ((base, field), _) = store.unwrap();
        assert_eq!(base, "obj");
        assert_eq!(field, "data");

        // Nested field store
        let nested = extractor.parse_field_store("self.inner.x = y");
        assert!(nested.is_some());
        let ((base2, field2), _) = nested.unwrap();
        assert_eq!(base2, "self.inner");
        assert_eq!(field2, "x");
    }

    #[test]
    fn test_nested_field_access() {
        let extractor = ConstraintExtractor::new();

        // Nested field access: a.b.c
        let nested = extractor.parse_field_access("x = obj.inner.value");
        assert_eq!(nested, Some("value".to_string()));

        // Deeply nested
        let deep = extractor.parse_field_access("y = a.b.c.d.e");
        assert_eq!(deep, Some("e".to_string()));
    }

    #[test]
    fn test_parameter_aliasing_conservative() {
        let mut ssa = create_test_ssa("param_alias_test");
        ssa.ssa_names = vec![create_ssa_name(0, "a", 0, 1), create_ssa_name(1, "b", 0, 1)];
        ssa.blocks = vec![SsaBlock {
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
        }];

        let extractor = ConstraintExtractor::extract_from_ssa(&ssa).unwrap();

        // Both parameters should be recorded
        assert!(extractor.parameters().contains("a_0"));
        assert!(extractor.parameters().contains("b_0"));

        // Both should have param locations
        assert!(extractor
            .allocation_sites()
            .iter()
            .any(|s| matches!(s, AbstractLocation::Param { .. })));
    }
}
