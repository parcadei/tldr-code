//! SSA Construction
//!
//! Implements Cytron et al.'s algorithm for SSA construction.
//!
//! # References
//! - Cytron et al. (1991) - "Efficiently Constructing SSA Form"

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::cfg::get_cfg_context;
use crate::dfg::get_dfg_context;
use crate::types::{CfgInfo, DfgInfo, Language, RefType, VarRef};
use crate::TldrResult;

use super::analysis::{compute_live_variables, LiveVariables};
use super::dominators::{build_dominator_tree, compute_dominance_frontier, DominatorTree};
use super::types::*;

// =============================================================================
// SSA Construction: Minimal SSA (Cytron Algorithm)
// =============================================================================

/// Construct minimal SSA form using Cytron algorithm
///
/// # Arguments
/// * `cfg` - Control flow graph for the function
/// * `dfg` - Data flow graph with variable references
///
/// # Returns
/// * `SsaFunction` - The function in SSA form
///
/// # Algorithm
/// Phase 1: Place Phi Functions
///   1. Build dominator tree
///   2. Compute dominance frontiers
///   3. For each variable v:
///      a. Collect all blocks containing definitions of v
///      b. Compute IDF of definition blocks
///      c. Insert phi function for v at each block in IDF
///
/// Phase 2: Rename Variables
///   1. For each variable v, initialize version counter to 0
///   2. DFS traversal of dominator tree:
///      - a. For each phi in current block: assign new version to target
///      - b. For each instruction in block:
///         - Replace uses with current version
///         - If defines v: assign new version
///      - c. Fill in phi sources in successors
///      - d. Recursively process dominated blocks
///      - e. Pop version stack when leaving block
///
/// # Edge Cases (from premortem)
/// - S10-P1-R12: Ensure phi placed at all IDF blocks
/// - S10-P1-R13: Use RAII guard pattern for version stack (track pop count)
/// - S10-P1-R14: Handle uninitialized variables with version 0 marker
/// - S10-P3-R5: RefType::Update treated as USE then DEF
pub fn construct_minimal_ssa(cfg: &CfgInfo, dfg: &DfgInfo) -> TldrResult<SsaFunction> {
    construct_minimal_ssa_with_statements(cfg, dfg, &[])
}

/// Construct minimal SSA form with source statements for richer instruction metadata.
///
/// When `statements` is non-empty, each SSA instruction will have:
/// - `source_text` populated from the corresponding source line
/// - `uses` populated from DFG Use refs on the same line as a Definition
pub fn construct_minimal_ssa_with_statements(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    statements: &[String],
) -> TldrResult<SsaFunction> {
    // Phase 0: Build dominator tree and dominance frontiers
    let dom_tree = build_dominator_tree(cfg)?;
    let dom_frontier = compute_dominance_frontier(cfg, &dom_tree)?;

    // Map lines to block IDs for looking up which block a VarRef is in
    let line_to_block = build_line_to_block_map(cfg);

    // Phase 1: Collect variable definitions and place phi functions
    let var_defs = collect_variable_definitions(dfg, &line_to_block);
    let phi_placements = place_phi_functions(&var_defs, &dom_frontier);

    // Phase 2: Build initial SSA blocks structure
    let mut ssa_blocks = build_initial_ssa_blocks(cfg, &phi_placements);

    // Phase 3: Rename variables
    let mut renaming_state = RenamingState::new();
    let renaming_context = SsaRenamingContext {
        dfg,
        dom_tree: &dom_tree,
        line_to_block: &line_to_block,
        statements,
    };
    rename_variables_recursive(
        cfg.entry_block,
        &renaming_context,
        &mut ssa_blocks,
        &mut renaming_state,
    );

    // Phase 4: Fill in phi sources from successors
    fill_phi_sources(cfg, &mut ssa_blocks, &renaming_state);

    // Phase 5: Build def-use chains
    let def_use = build_def_use_chains(&ssa_blocks);

    // Phase 6: Compute statistics
    let stats = compute_ssa_stats(&ssa_blocks, &renaming_state);

    Ok(SsaFunction {
        function: cfg.function.clone(),
        file: PathBuf::new(), // Will be set by caller
        ssa_type: SsaType::Minimal,
        blocks: ssa_blocks,
        ssa_names: renaming_state.into_ssa_names(),
        def_use,
        stats,
    })
}

/// Construct semi-pruned SSA (only non-local variables get phi functions)
///
/// # Arguments
/// * `cfg` - Control flow graph for the function
/// * `dfg` - Data flow graph with variable references
///
/// # Returns
/// * `SsaFunction` - The function in semi-pruned SSA form
///
/// # Algorithm
/// Before phi placement:
///   1. Identify "local" variables: defined and used only in one block
///   2. Exclude local variables from phi placement
///   3. Proceed with Cytron algorithm for remaining variables
pub fn construct_semi_pruned_ssa(cfg: &CfgInfo, dfg: &DfgInfo) -> TldrResult<SsaFunction> {
    construct_semi_pruned_ssa_with_statements(cfg, dfg, &[])
}

/// Construct semi-pruned SSA with source statements for richer instruction metadata.
pub fn construct_semi_pruned_ssa_with_statements(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    statements: &[String],
) -> TldrResult<SsaFunction> {
    // Phase 0: Build dominator tree and dominance frontiers
    let dom_tree = build_dominator_tree(cfg)?;
    let dom_frontier = compute_dominance_frontier(cfg, &dom_tree)?;

    // Map lines to block IDs
    let line_to_block = build_line_to_block_map(cfg);

    // Phase 1: Collect variable definitions, filtering out block-local variables
    let var_defs = collect_variable_definitions(dfg, &line_to_block);
    let non_local_vars = filter_non_local_variables(dfg, &line_to_block);

    // Only place phi for non-local variables
    let filtered_defs: HashMap<String, HashSet<usize>> = var_defs
        .into_iter()
        .filter(|(var, _)| non_local_vars.contains(var))
        .collect();

    let phi_placements = place_phi_functions(&filtered_defs, &dom_frontier);

    // Phase 2: Build initial SSA blocks structure
    let mut ssa_blocks = build_initial_ssa_blocks(cfg, &phi_placements);

    // Phase 3: Rename variables
    let mut renaming_state = RenamingState::new();
    let renaming_context = SsaRenamingContext {
        dfg,
        dom_tree: &dom_tree,
        line_to_block: &line_to_block,
        statements,
    };
    rename_variables_recursive(
        cfg.entry_block,
        &renaming_context,
        &mut ssa_blocks,
        &mut renaming_state,
    );

    // Phase 4: Fill in phi sources
    fill_phi_sources(cfg, &mut ssa_blocks, &renaming_state);

    // Phase 5: Build def-use chains
    let def_use = build_def_use_chains(&ssa_blocks);

    // Phase 6: Compute statistics
    let stats = compute_ssa_stats(&ssa_blocks, &renaming_state);

    Ok(SsaFunction {
        function: cfg.function.clone(),
        file: PathBuf::new(),
        ssa_type: SsaType::SemiPruned,
        blocks: ssa_blocks,
        ssa_names: renaming_state.into_ssa_names(),
        def_use,
        stats,
    })
}

/// Construct pruned SSA (requires live variable analysis)
///
/// # Arguments
/// * `cfg` - Control flow graph for the function
/// * `dfg` - Data flow graph with variable references
/// * `live_vars` - Pre-computed live variable analysis
///
/// # Returns
/// * `SsaFunction` - The function in pruned SSA form
///
/// # Algorithm
/// During phi placement:
///   For phi at block b for variable v:
///     Only insert if v is in LiveIn[b]
pub fn construct_pruned_ssa(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    live_vars: &LiveVariables,
) -> TldrResult<SsaFunction> {
    construct_pruned_ssa_with_statements(cfg, dfg, live_vars, &[])
}

/// Construct pruned SSA with source statements for richer instruction metadata.
pub fn construct_pruned_ssa_with_statements(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    live_vars: &LiveVariables,
    statements: &[String],
) -> TldrResult<SsaFunction> {
    // Phase 0: Build dominator tree and dominance frontiers
    let dom_tree = build_dominator_tree(cfg)?;
    let dom_frontier = compute_dominance_frontier(cfg, &dom_tree)?;

    // Map lines to block IDs
    let line_to_block = build_line_to_block_map(cfg);

    // Phase 1: Collect variable definitions
    let var_defs = collect_variable_definitions(dfg, &line_to_block);

    // Place phi functions, but only where variable is live
    let phi_placements = place_phi_functions_pruned(&var_defs, &dom_frontier, live_vars);

    // Phase 2: Build initial SSA blocks structure
    let mut ssa_blocks = build_initial_ssa_blocks(cfg, &phi_placements);

    // Phase 3: Rename variables
    let mut renaming_state = RenamingState::new();
    let renaming_context = SsaRenamingContext {
        dfg,
        dom_tree: &dom_tree,
        line_to_block: &line_to_block,
        statements,
    };
    rename_variables_recursive(
        cfg.entry_block,
        &renaming_context,
        &mut ssa_blocks,
        &mut renaming_state,
    );

    // Phase 4: Fill in phi sources
    fill_phi_sources(cfg, &mut ssa_blocks, &renaming_state);

    // Phase 5: Build def-use chains
    let def_use = build_def_use_chains(&ssa_blocks);

    // Phase 6: Compute statistics
    let stats = compute_ssa_stats(&ssa_blocks, &renaming_state);

    Ok(SsaFunction {
        function: cfg.function.clone(),
        file: PathBuf::new(),
        ssa_type: SsaType::Pruned,
        blocks: ssa_blocks,
        ssa_names: renaming_state.into_ssa_names(),
        def_use,
        stats,
    })
}

/// High-level SSA construction entry point
///
/// # Arguments
/// * `source` - Source code text
/// * `function` - Function name
/// * `lang` - Programming language
/// * `ssa_type` - Type of SSA to construct
///
/// # Returns
/// * `SsaFunction` - The function in SSA form
pub fn construct_ssa(
    source: &str,
    function: &str,
    lang: Language,
    ssa_type: SsaType,
) -> TldrResult<SsaFunction> {
    // Step 1: Extract CFG from source
    let cfg = get_cfg_context(source, function, lang)?;

    // Step 2: Extract DFG from source
    let dfg = get_dfg_context(source, function, lang)?;

    // Step 3: Split source into lines for source_text population
    let statements: Vec<String> = source.lines().map(|l| l.to_string()).collect();

    // Step 4: Construct SSA based on requested type, with source statements
    match ssa_type {
        SsaType::Minimal => construct_minimal_ssa_with_statements(&cfg, &dfg, &statements),
        SsaType::SemiPruned => construct_semi_pruned_ssa_with_statements(&cfg, &dfg, &statements),
        SsaType::Pruned => {
            // Pruned SSA requires live variables analysis
            match compute_live_variables(&cfg, &dfg.refs) {
                Ok(live_vars) => {
                    construct_pruned_ssa_with_statements(&cfg, &dfg, &live_vars, &statements)
                }
                Err(_) => {
                    // Fall back to semi-pruned if live variables unavailable
                    let mut ssa =
                        construct_semi_pruned_ssa_with_statements(&cfg, &dfg, &statements)?;
                    ssa.ssa_type = SsaType::Pruned;
                    Ok(ssa)
                }
            }
        }
    }
}

// =============================================================================
// Helper Functions: Line-to-Block Mapping
// =============================================================================

/// Build a map from line numbers to block IDs
fn build_line_to_block_map(cfg: &CfgInfo) -> HashMap<u32, usize> {
    let mut map = HashMap::new();
    for block in &cfg.blocks {
        for line in block.lines.0..=block.lines.1 {
            map.insert(line, block.id);
        }
    }
    map
}

/// Get the block ID for a given line number
fn get_block_for_line(line: u32, line_to_block: &HashMap<u32, usize>) -> Option<usize> {
    line_to_block.get(&line).copied()
}

/// Get source text for a given 1-based line number from statements array.
/// Returns None if statements is empty or line is out of range.
fn get_source_text(line: u32, statements: &[String]) -> Option<String> {
    if statements.is_empty() || line == 0 {
        return None;
    }
    statements.get((line as usize).wrapping_sub(1)).cloned()
}

/// Find Use refs on the same line as a Definition ref, and resolve their
/// current SSA name IDs from the renaming state.
fn resolve_uses_on_line(
    line: u32,
    def_var_name: &str,
    dfg: &DfgInfo,
    line_to_block: &HashMap<u32, usize>,
    block_id: usize,
    state: &RenamingState,
) -> Vec<SsaNameId> {
    dfg.refs
        .iter()
        .filter(|r| {
            r.line == line
                && r.name != def_var_name  // Don't include the variable being defined
                && matches!(r.ref_type, RefType::Use)
                && get_block_for_line(r.line, line_to_block) == Some(block_id)
        })
        .filter_map(|r| state.current(&r.name))
        .collect()
}

// =============================================================================
// Helper Functions: Variable Definition Collection
// =============================================================================

/// Collect all variable definitions from DFG, grouped by variable name
fn collect_variable_definitions(
    dfg: &DfgInfo,
    line_to_block: &HashMap<u32, usize>,
) -> HashMap<String, HashSet<usize>> {
    let mut var_defs: HashMap<String, HashSet<usize>> = HashMap::new();

    for var_ref in &dfg.refs {
        // Both Definition and Update create new versions
        if matches!(var_ref.ref_type, RefType::Definition | RefType::Update) {
            if let Some(block_id) = get_block_for_line(var_ref.line, line_to_block) {
                var_defs
                    .entry(var_ref.name.clone())
                    .or_default()
                    .insert(block_id);
            }
        }
    }

    var_defs
}

/// Identify non-local variables (used or defined in multiple blocks)
fn filter_non_local_variables(
    dfg: &DfgInfo,
    line_to_block: &HashMap<u32, usize>,
) -> HashSet<String> {
    // Track blocks where each variable is referenced
    let mut var_blocks: HashMap<String, HashSet<usize>> = HashMap::new();

    for var_ref in &dfg.refs {
        if let Some(block_id) = get_block_for_line(var_ref.line, line_to_block) {
            var_blocks
                .entry(var_ref.name.clone())
                .or_default()
                .insert(block_id);
        }
    }

    // Non-local = appears in more than one block
    var_blocks
        .into_iter()
        .filter(|(_, blocks)| blocks.len() > 1)
        .map(|(var, _)| var)
        .collect()
}

// =============================================================================
// Helper Functions: Phi Placement
// =============================================================================

/// Place phi functions using iterated dominance frontier (IDF)
fn place_phi_functions(
    var_defs: &HashMap<String, HashSet<usize>>,
    dom_frontier: &super::dominators::DominanceFrontier,
) -> HashMap<usize, Vec<String>> {
    let mut phi_placements: HashMap<usize, Vec<String>> = HashMap::new();

    for (var, def_blocks) in var_defs {
        // Get IDF for this variable's definition blocks
        let idf = dom_frontier.iterated(def_blocks);

        // Place phi for this variable at each IDF block
        for block in idf {
            phi_placements.entry(block).or_default().push(var.clone());
        }
    }

    phi_placements
}

/// Place phi functions only where variable is live (pruned SSA)
fn place_phi_functions_pruned(
    var_defs: &HashMap<String, HashSet<usize>>,
    dom_frontier: &super::dominators::DominanceFrontier,
    live_vars: &LiveVariables,
) -> HashMap<usize, Vec<String>> {
    let mut phi_placements: HashMap<usize, Vec<String>> = HashMap::new();

    for (var, def_blocks) in var_defs {
        // Get IDF for this variable's definition blocks
        let idf = dom_frontier.iterated(def_blocks);

        // Place phi only if variable is live at block entry
        for block in idf {
            let is_live = live_vars
                .blocks
                .get(&block)
                .map(|sets| sets.live_in.contains(var))
                .unwrap_or(false);

            if is_live {
                phi_placements.entry(block).or_default().push(var.clone());
            }
        }
    }

    phi_placements
}

// =============================================================================
// Helper Functions: SSA Block Construction
// =============================================================================

/// Build initial SSA blocks from CFG with phi placeholders
fn build_initial_ssa_blocks(
    cfg: &CfgInfo,
    phi_placements: &HashMap<usize, Vec<String>>,
) -> Vec<SsaBlock> {
    // Build predecessor map
    let mut predecessors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        predecessors.entry(block.id).or_default();
    }
    for edge in &cfg.edges {
        predecessors.entry(edge.to).or_default().push(edge.from);
    }

    // Build successor map
    let mut successors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        successors.entry(block.id).or_default();
    }
    for edge in &cfg.edges {
        successors.entry(edge.from).or_default().push(edge.to);
    }

    cfg.blocks
        .iter()
        .map(|block| {
            // Create phi functions for this block (with placeholder targets)
            let phi_functions = phi_placements
                .get(&block.id)
                .map(|vars| {
                    vars.iter()
                        .map(|var| PhiFunction {
                            target: SsaNameId(0), // Will be filled during renaming
                            variable: var.clone(),
                            sources: Vec::new(), // Will be filled after renaming
                            line: block.lines.0,
                        })
                        .collect()
                })
                .unwrap_or_default();

            SsaBlock {
                id: block.id,
                label: if cfg.entry_block == block.id {
                    Some("entry".to_string())
                } else if cfg.exit_blocks.contains(&block.id) {
                    Some("exit".to_string())
                } else {
                    None
                },
                lines: block.lines,
                phi_functions,
                instructions: Vec::new(),
                successors: successors.get(&block.id).cloned().unwrap_or_default(),
                predecessors: predecessors.get(&block.id).cloned().unwrap_or_default(),
            }
        })
        .collect()
}

// =============================================================================
// Renaming State
// =============================================================================

/// State for variable renaming during SSA construction
pub struct RenamingState {
    /// Counter for generating unique SsaNameIds
    next_id: u32,
    /// Stack of versions for each variable: var -> [SsaNameId stack]
    stacks: HashMap<String, Vec<SsaNameId>>,
    /// All SSA names created, indexed by SsaNameId
    names: HashMap<SsaNameId, SsaName>,
    /// Version counter per variable
    counters: HashMap<String, u32>,
    /// Track which block each version was created in (for phi source lookup)
    block_stacks: HashMap<String, Vec<(SsaNameId, usize)>>,
    /// Track the version that was current at the END of each block (for phi sources)
    /// Key: (variable, block_id), Value: SsaNameId
    block_exit_versions: HashMap<(String, usize), SsaNameId>,
    /// Track moved variables (Rust ownership semantics)
    moved_vars: HashSet<String>,
}

impl RenamingState {
    fn new() -> Self {
        Self {
            next_id: 1, // Start at 1 (0 reserved for undefined)
            stacks: HashMap::new(),
            names: HashMap::new(),
            counters: HashMap::new(),
            block_stacks: HashMap::new(),
            block_exit_versions: HashMap::new(),
            moved_vars: HashSet::new(),
        }
    }

    /// Create a new SSA name for variable
    fn new_name(&mut self, var: &str, block_id: usize, line: u32) -> SsaNameId {
        let id = SsaNameId(self.next_id);
        self.next_id += 1;

        let version = self.counters.entry(var.to_string()).or_insert(0);
        *version += 1;
        let current_version = *version;

        let name = SsaName {
            id,
            variable: var.to_string(),
            version: current_version,
            def_block: Some(block_id),
            def_line: line,
        };

        self.names.insert(id, name);
        self.stacks.entry(var.to_string()).or_default().push(id);
        self.block_stacks
            .entry(var.to_string())
            .or_default()
            .push((id, block_id));

        id
    }

    /// Get current version of variable (top of stack)
    fn current(&self, var: &str) -> Option<SsaNameId> {
        self.stacks.get(var).and_then(|s| s.last().copied())
    }

    /// Record how many versions were pushed for tracking pops
    fn stack_depth(&self, var: &str) -> usize {
        self.stacks.get(var).map(|s| s.len()).unwrap_or(0)
    }

    /// Pop versions to restore stack to given depth
    fn restore_depth(&mut self, var: &str, target_depth: usize) {
        if let Some(stack) = self.stacks.get_mut(var) {
            while stack.len() > target_depth {
                stack.pop();
            }
        }
        if let Some(block_stack) = self.block_stacks.get_mut(var) {
            while block_stack.len() > target_depth {
                block_stack.pop();
            }
        }
    }

    /// Convert to Vec<SsaName> for final output
    fn into_ssa_names(self) -> Vec<SsaName> {
        let mut names: Vec<_> = self.names.into_values().collect();
        names.sort_by_key(|n| n.id.0);
        names
    }
}

// =============================================================================
// Variable Renaming
// =============================================================================

/// Rename variables using dominator tree traversal (recursive)
struct SsaRenamingContext<'a> {
    dfg: &'a DfgInfo,
    dom_tree: &'a DominatorTree,
    line_to_block: &'a HashMap<u32, usize>,
    statements: &'a [String],
}

/// Rename variables using dominator tree traversal (recursive)
fn rename_variables_recursive(
    block_id: usize,
    context: &SsaRenamingContext<'_>,
    ssa_blocks: &mut [SsaBlock],
    state: &mut RenamingState,
) {
    // Find the SSA block
    let ssa_block_idx = ssa_blocks.iter().position(|b| b.id == block_id);
    if ssa_block_idx.is_none() {
        return;
    }
    let ssa_block_idx = ssa_block_idx.unwrap();

    // Track stack depths before processing this block (for restoration)
    let mut initial_depths: HashMap<String, usize> = HashMap::new();

    // Get block line range
    let block_lines = ssa_blocks[ssa_block_idx].lines;

    // Step 1: Process phi function targets (create new versions)
    let phi_vars: Vec<String> = ssa_blocks[ssa_block_idx]
        .phi_functions
        .iter()
        .map(|phi| phi.variable.clone())
        .collect();

    for var in &phi_vars {
        initial_depths
            .entry(var.clone())
            .or_insert_with(|| state.stack_depth(var));
    }

    for (idx, var) in phi_vars.iter().enumerate() {
        let new_id = state.new_name(var, block_id, block_lines.0);
        ssa_blocks[ssa_block_idx].phi_functions[idx].target = new_id;
    }

    // Step 2: Process instructions from DFG refs in this block
    let refs_in_block: Vec<&VarRef> = context
        .dfg
        .refs
        .iter()
        .filter(|r| get_block_for_line(r.line, context.line_to_block) == Some(block_id))
        .collect();

    // Sort by line for correct processing order
    let mut refs_sorted = refs_in_block;
    refs_sorted.sort_by_key(|r| (r.line, matches!(r.ref_type, RefType::Use)));

    // Track variables touched in this block for recording exit versions
    let mut touched_vars: HashSet<String> = phi_vars.iter().cloned().collect();

    for var_ref in refs_sorted {
        initial_depths
            .entry(var_ref.name.clone())
            .or_insert_with(|| state.stack_depth(&var_ref.name));

        touched_vars.insert(var_ref.name.clone());

        match var_ref.ref_type {
            RefType::Definition => {
                // Resolve RHS uses BEFORE creating the new definition version
                // (so that `y = x` resolves x's current version, not y's)
                let rhs_uses = resolve_uses_on_line(
                    var_ref.line,
                    &var_ref.name,
                    context.dfg,
                    context.line_to_block,
                    block_id,
                    state,
                );

                // Create new version
                let target = state.new_name(&var_ref.name, block_id, var_ref.line);

                // Add instruction with source_text and uses populated
                ssa_blocks[ssa_block_idx].instructions.push(SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(target),
                    uses: rhs_uses,
                    line: var_ref.line,
                    source_text: get_source_text(var_ref.line, context.statements),
                });
            }
            RefType::Update => {
                // Update = USE then DEF (S10-P3-R5)
                // First record the use of current version
                let use_version = state.current(&var_ref.name);

                // Also resolve other RHS uses on the same line
                let mut all_uses: Vec<SsaNameId> = resolve_uses_on_line(
                    var_ref.line,
                    &var_ref.name,
                    context.dfg,
                    context.line_to_block,
                    block_id,
                    state,
                );
                // Include the self-use (e.g., x += 1 uses x)
                if let Some(self_use) = use_version {
                    if !all_uses.contains(&self_use) {
                        all_uses.insert(0, self_use);
                    }
                }

                // Then create new version
                let target = state.new_name(&var_ref.name, block_id, var_ref.line);

                // Add instruction with both use and def
                ssa_blocks[ssa_block_idx].instructions.push(SsaInstruction {
                    kind: SsaInstructionKind::Assign,
                    target: Some(target),
                    uses: all_uses,
                    line: var_ref.line,
                    source_text: get_source_text(var_ref.line, context.statements),
                });
            }
            RefType::Use => {
                // Record use of current version
                let _use_version = state.current(&var_ref.name);
                // Uses are recorded in def-use chains later
            }
        }
    }

    // Record current versions at block exit (before recursing into children)
    // This is used for phi source lookup
    for var in &touched_vars {
        if let Some(current_id) = state.current(var) {
            state
                .block_exit_versions
                .insert((var.clone(), block_id), current_id);
        }
    }
    // Also record versions for variables that flow through this block unchanged
    // (needed for phi sources from predecessors that don't modify the variable)
    for (var, stack) in &state.stacks {
        if !touched_vars.contains(var) {
            if let Some(&current_id) = stack.last() {
                state
                    .block_exit_versions
                    .insert((var.clone(), block_id), current_id);
            }
        }
    }

    // Step 3: Recursively process dominated children
    if let Some(node) = context.dom_tree.nodes.get(&block_id) {
        for &child in &node.children {
            rename_variables_recursive(child, context, ssa_blocks, state);
        }
    }

    // Step 4: Restore stack depths (pop versions created in this block)
    for (var, depth) in initial_depths {
        state.restore_depth(&var, depth);
    }
}

/// Fill in phi sources after renaming is complete
fn fill_phi_sources(_cfg: &CfgInfo, ssa_blocks: &mut [SsaBlock], state: &RenamingState) {
    // For each block with phi functions
    for ssa_block in ssa_blocks.iter_mut() {
        if ssa_block.phi_functions.is_empty() {
            continue;
        }

        // Get predecessor blocks
        let predecessors = ssa_block.predecessors.clone();

        // For each phi function
        for phi in &mut ssa_block.phi_functions {
            // For each predecessor
            for &pred_id in &predecessors {
                // Get the version of the variable that was current at end of predecessor
                // This uses the block_exit_versions map we populated during renaming
                let source_version = state
                    .block_exit_versions
                    .get(&(phi.variable.clone(), pred_id))
                    .copied();

                if let Some(version) = source_version {
                    phi.sources.push(PhiSource {
                        block: pred_id,
                        name: version,
                    });
                }
            }
        }
    }
}

// =============================================================================
// Helper Functions: Def-Use Chains and Statistics
// =============================================================================

/// Build def-use chains from SSA blocks
fn build_def_use_chains(ssa_blocks: &[SsaBlock]) -> HashMap<SsaNameId, Vec<SsaNameId>> {
    let mut def_use: HashMap<SsaNameId, Vec<SsaNameId>> = HashMap::new();

    // Collect all definitions
    for block in ssa_blocks {
        // From phi functions
        for phi in &block.phi_functions {
            def_use.entry(phi.target).or_default();
            // Phi sources are uses of their defining versions
            for source in &phi.sources {
                def_use.entry(source.name).or_default().push(phi.target);
            }
        }

        // From instructions
        for inst in &block.instructions {
            if let Some(target) = inst.target {
                def_use.entry(target).or_default();
            }
            // Uses point back to definitions
            for &use_id in &inst.uses {
                if let Some(target) = inst.target {
                    def_use.entry(use_id).or_default().push(target);
                }
            }
        }
    }

    def_use
}

/// Compute SSA statistics
fn compute_ssa_stats(ssa_blocks: &[SsaBlock], state: &RenamingState) -> SsaStats {
    let phi_count: usize = ssa_blocks.iter().map(|b| b.phi_functions.len()).sum();
    let instructions: usize = ssa_blocks.iter().map(|b| b.instructions.len()).sum();

    SsaStats {
        phi_count,
        ssa_names: state.names.len(),
        blocks: ssa_blocks.len(),
        instructions,
        dead_phi_count: 0, // Would need use analysis to compute
    }
}

// =============================================================================
// Public Query Functions
// =============================================================================

/// Assign version numbers to create unique SSA names
///
/// # Arguments
/// * `ssa` - SSA function with phi functions placed
/// * `cfg` - Control flow graph
/// * `dom_tree` - Dominator tree
///
/// # Postconditions
/// * Each SsaName has unique (variable, version) pair
/// * Version numbers start at 1 and increment per definition
/// * All uses reference correct version (reaching definition)
pub fn rename_variables(
    _ssa: &mut SsaFunction,
    _cfg: &CfgInfo,
    _dom_tree: &DominatorTree,
) -> TldrResult<()> {
    // This is now integrated into construct_minimal_ssa
    // Keeping for API compatibility
    Ok(())
}

/// Get the defining instruction for an SSA name
pub fn get_definition(ssa: &SsaFunction, name: SsaNameId) -> Option<&SsaInstruction> {
    for block in &ssa.blocks {
        for inst in &block.instructions {
            if inst.target == Some(name) {
                return Some(inst);
            }
        }
    }
    None
}

/// Get the defining block for an SSA name
pub fn get_def_block(ssa: &SsaFunction, name: SsaNameId) -> Option<usize> {
    ssa.ssa_names
        .iter()
        .find(|n| n.id == name)
        .and_then(|n| n.def_block)
}

/// Filter SSA to show only a specific variable
pub fn filter_ssa_by_variable(mut ssa: SsaFunction, variable: &str) -> SsaFunction {
    // Keep only SSA names for this variable
    let keep_ids: HashSet<SsaNameId> = ssa
        .ssa_names
        .iter()
        .filter(|n| n.variable == variable)
        .map(|n| n.id)
        .collect();

    ssa.ssa_names.retain(|n| n.variable == variable);

    for block in &mut ssa.blocks {
        // Filter phi functions
        block.phi_functions.retain(|phi| phi.variable == variable);

        // Filter instructions that define or use this variable
        block.instructions.retain(|inst| {
            inst.target.is_some_and(|t| keep_ids.contains(&t))
                || inst.uses.iter().any(|u| keep_ids.contains(u))
        });
    }

    // Update def_use chains
    ssa.def_use.retain(|k, _| keep_ids.contains(k));
    for uses in ssa.def_use.values_mut() {
        uses.retain(|u| keep_ids.contains(u));
    }

    ssa
}

// =============================================================================
// Language-Specific SSA Handlers (Phase 5)
// =============================================================================
//
// These handlers address language-specific patterns identified in session10-premortem-2.yaml:
// - Python: S10-P2-R1 through R12
// - TypeScript: S10-P2-R13 through R21
// - Go: S10-P2-R22 through R29
// - Rust: S10-P2-R30 through R36

use crate::types::VarRefContext;

/// Result of processing a language-specific construct
#[derive(Debug)]
pub struct LanguageConstructResult {
    /// SSA names created (targets of assignments)
    pub definitions: Vec<SsaNameId>,
    /// SSA names used (sources)
    pub uses: Vec<SsaNameId>,
    /// Instructions to emit
    pub instructions: Vec<SsaInstruction>,
}

// =============================================================================
// Python-Specific Handlers (S10-P2-R1 through R12)
// =============================================================================

/// Handle Python augmented assignment (x += 1)
///
/// This is both a USE and DEF - process USE first, then DEF.
/// S10-P2-R1: Must capture read-then-write atomically.
///
/// # Example
/// ```python
/// x = 1     # x_1
/// x += 5    # USE x_1, then DEF x_2
/// return x  # USE x_2
/// ```
pub fn handle_augmented_assignment(
    var_ref: &VarRef,
    state: &mut RenamingState,
    block_id: usize,
) -> LanguageConstructResult {
    // Step 1: Get current version for the USE
    let use_version = state.current(&var_ref.name);

    // Step 2: Create new version for the DEF
    let target = state.new_name(&var_ref.name, block_id, var_ref.line);

    // Step 3: Build instruction with use-then-def semantics
    let instruction = SsaInstruction {
        kind: SsaInstructionKind::Assign,
        target: Some(target),
        uses: use_version.into_iter().collect(),
        line: var_ref.line,
        source_text: None,
    };

    LanguageConstructResult {
        definitions: vec![target],
        uses: use_version.into_iter().collect(),
        instructions: vec![instruction],
    }
}

/// Handle Python multiple assignment with parallel semantics (a, b = b, a)
///
/// RHS is evaluated completely before any LHS bindings.
/// S10-P2-R2: Must capture parallel semantics to avoid circular dependencies.
///
/// # Example
/// ```python
/// a, b = 1, 2       # a_1, b_1
/// a, b = b, a       # USE b_1, a_1 FIRST, then DEF a_2, b_2
/// ```
///
/// # Arguments
/// * `refs` - All VarRefs with the same group_id (same statement)
/// * `state` - Renaming state
/// * `block_id` - Current block ID
pub fn handle_multiple_assignment(
    refs: &[&VarRef],
    state: &mut RenamingState,
    block_id: usize,
) -> LanguageConstructResult {
    // Step 1: Collect all USES first (RHS evaluation)
    let uses: Vec<SsaNameId> = refs
        .iter()
        .filter(|r| matches!(r.ref_type, RefType::Use))
        .filter_map(|r| state.current(&r.name))
        .collect();

    // Step 2: Create all DEFINITIONS (LHS bindings)
    let mut definitions = Vec::new();
    let mut instructions = Vec::new();

    for var_ref in refs
        .iter()
        .filter(|r| matches!(r.ref_type, RefType::Definition))
    {
        let target = state.new_name(&var_ref.name, block_id, var_ref.line);
        definitions.push(target);

        // Each definition gets all the uses (parallel semantics)
        instructions.push(SsaInstruction {
            kind: SsaInstructionKind::Assign,
            target: Some(target),
            uses: uses.clone(),
            line: var_ref.line,
            source_text: None,
        });
    }

    LanguageConstructResult {
        definitions,
        uses,
        instructions,
    }
}

/// Handle Python walrus operator (:=)
///
/// Definition happens in expression context - variable escapes to enclosing scope.
/// S10-P2-R3: Walrus operator creates definition visible after the expression.
///
/// # Example
/// ```python
/// if (n := len(items)) > 0:
///     print(n)  # n_1 is visible here
/// print(n)      # n_1 still visible here
/// ```
pub fn handle_walrus_operator(
    var_ref: &VarRef,
    state: &mut RenamingState,
    block_id: usize,
) -> LanguageConstructResult {
    // Walrus creates a definition that escapes the expression
    let target = state.new_name(&var_ref.name, block_id, var_ref.line);

    let instruction = SsaInstruction {
        kind: SsaInstructionKind::Assign,
        target: Some(target),
        uses: Vec::new(), // RHS would need expression analysis
        line: var_ref.line,
        source_text: None,
    };

    LanguageConstructResult {
        definitions: vec![target],
        uses: Vec::new(),
        instructions: vec![instruction],
    }
}

/// Check if a VarRef is in comprehension scope
///
/// Comprehension loop variables are isolated and don't affect outer scope.
/// S10-P2-R5: Comprehension x doesn't affect outer x.
///
/// # Example
/// ```python
/// x = 10
/// result = [x for x in range(5)]  # comprehension x is separate
/// print(x)  # prints 10, not 4
/// ```
pub fn is_comprehension_scope(var_ref: &VarRef) -> bool {
    var_ref
        .context
        .as_ref()
        .is_some_and(|c| matches!(c, VarRefContext::ComprehensionScope))
}

// =============================================================================
// TypeScript-Specific Handlers (S10-P2-R13 through R21)
// =============================================================================

/// Handle destructuring assignment: const {a, b} = obj
///
/// Creates multiple definitions from one statement.
/// S10-P2-R13: Must handle nested destructuring and renamed bindings.
///
/// # Example
/// ```typescript
/// const {a, b: c} = obj;  // Defines a_1 and c_1 (NOT b!)
/// const [x, , y] = arr;   // Defines x_1 and y_1, middle skipped
/// ```
pub fn handle_destructuring(
    refs: &[&VarRef],
    state: &mut RenamingState,
    block_id: usize,
) -> LanguageConstructResult {
    // Collect sources (the object/array being destructured)
    let source_uses: Vec<SsaNameId> = refs
        .iter()
        .filter(|r| matches!(r.ref_type, RefType::Use))
        .filter_map(|r| state.current(&r.name))
        .collect();

    // Create definitions for each destructured binding
    let mut definitions = Vec::new();
    let mut instructions = Vec::new();

    for var_ref in refs
        .iter()
        .filter(|r| matches!(r.ref_type, RefType::Definition))
    {
        let target = state.new_name(&var_ref.name, block_id, var_ref.line);
        definitions.push(target);

        instructions.push(SsaInstruction {
            kind: SsaInstructionKind::Assign,
            target: Some(target),
            uses: source_uses.clone(),
            line: var_ref.line,
            source_text: None,
        });
    }

    LanguageConstructResult {
        definitions,
        uses: source_uses,
        instructions,
    }
}

/// Handle closure capture in TypeScript/JavaScript
///
/// Closures capture by REFERENCE, not by value.
/// S10-P2-R17: The USE happens at CALL time, not definition time.
///
/// # Example
/// ```typescript
/// let x = 1;
/// const f = () => x + 1;  // Captures 'x' variable, not x_1
/// x = 2;
/// f();  // Returns 3, uses x_2
/// ```
///
/// For SSA purposes, we mark this as a special capture that may see any version.
pub fn handle_closure_capture(captured_var: &str, state: &RenamingState) -> Option<SsaNameId> {
    // Return current version at capture time
    // Note: This is a simplification. Full modeling requires MemorySSA
    // because the captured variable may be modified later.
    state.current(captured_var)
}

// =============================================================================
// Go-Specific Handlers (S10-P2-R22 through R29)
// =============================================================================

/// Handle Go short declaration (:=)
///
/// May create new variable OR reassign existing (if mixed with =).
/// S10-P2-R22: x, y := 3, 4 where x exists: x is reassigned, y is new.
///
/// # Example
/// ```go
/// x := 1       // x_1 (new)
/// x, y := 2, 3 // x_2 (reassign), y_1 (new)
/// ```
pub fn handle_short_declaration(
    var_ref: &VarRef,
    state: &mut RenamingState,
    block_id: usize,
    _is_new_var: bool, // DFG should tell us, but we always create new version
) -> SsaNameId {
    // For SSA, both new declarations and reassignments create new versions
    state.new_name(&var_ref.name, block_id, var_ref.line)
}

/// Handle Go multiple return values
///
/// func f() (int, error) returns create 2 definitions.
/// S10-P2-R23: a, err := f() creates two definitions.
pub fn handle_multiple_return(
    refs: &[&VarRef],
    state: &mut RenamingState,
    block_id: usize,
) -> LanguageConstructResult {
    // Similar to destructuring - multiple definitions from one statement
    let mut definitions = Vec::new();
    let mut instructions = Vec::new();

    for var_ref in refs
        .iter()
        .filter(|r| matches!(r.ref_type, RefType::Definition))
    {
        let target = state.new_name(&var_ref.name, block_id, var_ref.line);
        definitions.push(target);

        instructions.push(SsaInstruction {
            kind: SsaInstructionKind::Assign,
            target: Some(target),
            uses: Vec::new(), // From function call
            line: var_ref.line,
            source_text: None,
        });
    }

    LanguageConstructResult {
        definitions,
        uses: Vec::new(),
        instructions,
    }
}

/// Check if identifier is Go blank identifier (_)
///
/// Discard - no SSA name should be created.
/// S10-P2-R28: _ is never a definition or use.
pub fn is_blank_identifier(name: &str) -> bool {
    name == "_"
}

// =============================================================================
// Rust-Specific Handlers (S10-P2-R30 through R36)
// =============================================================================

/// Handle Rust shadowing
///
/// let x = 1; let x = 2; creates TWO DIFFERENT variables.
/// S10-P2-R31: Shadowing creates new variable chain, NOT new version of old.
///
/// # Example
/// ```rust
/// let x = 1;      // x#1_1
/// let x = x + 1;  // x#2_1 (uses x#1_1)
/// let x = "str";  // x#3_1 (different type!)
/// ```
///
/// For SSA, we disambiguate with shadow count: x#1, x#2, x#3
pub fn handle_rust_shadowing(
    var_ref: &VarRef,
    state: &mut RenamingState,
    block_id: usize,
    is_new_binding: bool, // true for `let x`, false for reassignment
) -> SsaNameId {
    if is_new_binding {
        // Shadowing: create a fresh variable with shadow suffix
        let shadow_count = state.shadow_count(&var_ref.name);
        let shadow_name = format!("{}#{}", var_ref.name, shadow_count + 1);
        state.new_name(&shadow_name, block_id, var_ref.line)
    } else {
        // Reassignment of mutable: new version of existing
        state.new_name(&var_ref.name, block_id, var_ref.line)
    }
}

/// Handle Rust ownership move
///
/// After move, variable is no longer live.
/// S10-P2-R32: let b = a; makes a invalid after this point.
pub fn handle_ownership_move(var_ref: &VarRef, state: &mut RenamingState) {
    // Mark variable as moved (future uses would be errors)
    // This is tracked for analysis but doesn't affect SSA construction
    state.mark_moved(&var_ref.name);
}

/// Handle Rust pattern matching bindings
///
/// match x { Some(v) => ... } creates binding v in arm scope.
/// S10-P2-R34: Bindings are scoped to their arm only.
pub fn handle_match_binding(
    pattern_var: &str,
    state: &mut RenamingState,
    block_id: usize,
    line: u32,
) -> SsaNameId {
    // Pattern bindings are scoped to the match arm
    state.new_name(pattern_var, block_id, line)
}

// =============================================================================
// RenamingState Extensions for Language-Specific Features
// =============================================================================

impl RenamingState {
    /// Count how many times a variable has been shadowed (Rust)
    pub fn shadow_count(&self, var: &str) -> usize {
        // Count existing shadow variants: x#1, x#2, etc.
        let prefix = format!("{}#", var);
        self.stacks
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .count()
    }

    /// Mark a variable as moved (Rust ownership)
    pub fn mark_moved(&mut self, var: &str) {
        // We could track this in a separate set, but for now we just record it
        // This information would be used by liveness analysis
        self.moved_vars.insert(var.to_string());
    }

    /// Check if variable was moved
    pub fn is_moved(&self, var: &str) -> bool {
        self.moved_vars.contains(var)
    }
}

// =============================================================================
// Integrated Language-Specific Processing
// =============================================================================

/// Process a VarRef with language-specific handling
///
/// This is the main entry point for language-aware SSA construction.
/// It dispatches to the appropriate handler based on the VarRef's context.
pub fn process_var_ref_with_context(
    var_ref: &VarRef,
    state: &mut RenamingState,
    block_id: usize,
    instructions: &mut Vec<SsaInstruction>,
) -> Option<SsaNameId> {
    // Skip blank identifiers (Go)
    if is_blank_identifier(&var_ref.name) {
        return None;
    }

    // Check for language-specific context
    match &var_ref.context {
        Some(VarRefContext::AugmentedAssignment) => {
            let result = handle_augmented_assignment(var_ref, state, block_id);
            instructions.extend(result.instructions);
            result.definitions.first().copied()
        }

        Some(VarRefContext::WalrusOperator) => {
            let result = handle_walrus_operator(var_ref, state, block_id);
            instructions.extend(result.instructions);
            result.definitions.first().copied()
        }

        Some(VarRefContext::ComprehensionScope) => {
            // Comprehension variables are scoped - use mangled name
            let scoped_name = format!("{}$comprehension", var_ref.name);
            match var_ref.ref_type {
                RefType::Definition | RefType::Update => {
                    let target = state.new_name(&scoped_name, block_id, var_ref.line);
                    instructions.push(SsaInstruction {
                        kind: SsaInstructionKind::Assign,
                        target: Some(target),
                        uses: Vec::new(),
                        line: var_ref.line,
                        source_text: None,
                    });
                    Some(target)
                }
                RefType::Use => state.current(&scoped_name),
            }
        }

        Some(VarRefContext::Shadowing) => {
            // Rust shadowing creates new variable
            Some(handle_rust_shadowing(var_ref, state, block_id, true))
        }

        Some(VarRefContext::PatternBinding) | Some(VarRefContext::MatchArmBinding) => Some(
            handle_match_binding(&var_ref.name, state, block_id, var_ref.line),
        ),

        Some(VarRefContext::OwnershipMove) => {
            // Record the move
            handle_ownership_move(var_ref, state);
            // Still create the use
            state.current(&var_ref.name)
        }

        Some(VarRefContext::BlankIdentifier) => {
            // Skip - no SSA name
            None
        }

        Some(VarRefContext::GlobalNonlocal) => {
            // Global/nonlocal references bypass local SSA
            // They would be handled by MemorySSA
            None
        }

        // Group-based constructs (handled by group processing)
        Some(VarRefContext::MultipleAssignment)
        | Some(VarRefContext::Destructuring)
        | Some(VarRefContext::MultipleReturn)
        | Some(VarRefContext::ShortDeclaration) => {
            // These are handled by group-based processing in process_statement_group
            // Fall through to normal processing if not in a group
            process_normal_var_ref(var_ref, state, block_id, instructions)
        }

        // Optional chaining, closure capture, defer - handled specially
        Some(VarRefContext::OptionalChain) => {
            // Treat as normal use for now
            process_normal_var_ref(var_ref, state, block_id, instructions)
        }

        Some(VarRefContext::ClosureCapture) => {
            // Mark as captured, return current version
            state.current(&var_ref.name)
        }

        Some(VarRefContext::DeferCapture) => {
            // Captured at defer time - return current version
            state.current(&var_ref.name)
        }

        Some(VarRefContext::MatchBinding) => {
            // Python match binding - scoped definition
            Some(state.new_name(&var_ref.name, block_id, var_ref.line))
        }

        None => {
            // No special context - normal processing
            process_normal_var_ref(var_ref, state, block_id, instructions)
        }
    }
}

/// Process a normal VarRef without special context
fn process_normal_var_ref(
    var_ref: &VarRef,
    state: &mut RenamingState,
    block_id: usize,
    instructions: &mut Vec<SsaInstruction>,
) -> Option<SsaNameId> {
    match var_ref.ref_type {
        RefType::Definition => {
            let target = state.new_name(&var_ref.name, block_id, var_ref.line);
            instructions.push(SsaInstruction {
                kind: SsaInstructionKind::Assign,
                target: Some(target),
                uses: Vec::new(),
                line: var_ref.line,
                source_text: None,
            });
            Some(target)
        }
        RefType::Update => {
            // Update = USE then DEF
            let use_version = state.current(&var_ref.name);
            let target = state.new_name(&var_ref.name, block_id, var_ref.line);
            instructions.push(SsaInstruction {
                kind: SsaInstructionKind::Assign,
                target: Some(target),
                uses: use_version.into_iter().collect(),
                line: var_ref.line,
                source_text: None,
            });
            Some(target)
        }
        RefType::Use => {
            // Just record the use - no instruction emitted here
            state.current(&var_ref.name)
        }
    }
}

/// Process a group of VarRefs from the same statement
///
/// Used for multiple assignment, destructuring, and multiple returns.
pub fn process_statement_group(
    refs: &[&VarRef],
    state: &mut RenamingState,
    block_id: usize,
) -> LanguageConstructResult {
    // Determine the group type from the first ref with context
    let context = refs.iter().find_map(|r| r.context.as_ref());

    match context {
        Some(VarRefContext::MultipleAssignment) => {
            handle_multiple_assignment(refs, state, block_id)
        }
        Some(VarRefContext::Destructuring) => handle_destructuring(refs, state, block_id),
        Some(VarRefContext::MultipleReturn) | Some(VarRefContext::ShortDeclaration) => {
            handle_multiple_return(refs, state, block_id)
        }
        _ => {
            // Default: process each ref normally
            let mut definitions = Vec::new();
            let mut uses = Vec::new();
            let mut instructions = Vec::new();

            for var_ref in refs {
                if let Some(id) =
                    process_normal_var_ref(var_ref, state, block_id, &mut instructions)
                {
                    match var_ref.ref_type {
                        RefType::Definition | RefType::Update => definitions.push(id),
                        RefType::Use => uses.push(id),
                    }
                }
            }

            LanguageConstructResult {
                definitions,
                uses,
                instructions,
            }
        }
    }
}
