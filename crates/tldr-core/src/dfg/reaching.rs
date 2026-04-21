//! Reaching Definitions Analysis
//!
//! Classic dataflow analysis to determine which variable definitions
//! can reach each use point.
//!
//! # Algorithm
//!
//! For each basic block B:
//! - GEN[B] = definitions generated in B (last def of each var)
//! - KILL[B] = definitions killed by B (defs of vars also defined in B)
//!
//! Iterative algorithm:
//! ```text
//! IN[B] = union(OUT[P]) for all predecessors P
//! OUT[B] = GEN[B] union (IN[B] - KILL[B])
//! ```
//!
//! Repeat until no changes (fixed point).
//!
//! # Complexity
//! O(n * d) where n = number of blocks, d = number of definitions
//! Usually converges in 2-3 iterations for acyclic CFGs.
//!
//! # Enhanced Capabilities (Phase 8)
//!
//! - **Def-Use Chains (RD-7)**: For each definition, find all uses it reaches
//! - **Use-Def Chains (RD-8)**: For each use, find all definitions reaching it

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::types::{CfgInfo, RefType, VarRef};

// Re-export chain types from chains module for convenience
pub use super::chains::{
    BlockReachingDefs, DefUseChain, Definition, ReachingDefsReport, ReachingDefsStats,
    UninitSeverity, UninitializedUse, Use, UseDefChain,
};

/// Result of reaching definitions analysis
#[derive(Debug, Clone)]
pub struct ReachingDefinitions {
    /// For each block ID, the set of definitions that reach it (IN set)
    pub reaching_in: HashMap<usize, HashSet<DefId>>,
    /// For each block ID, the set of definitions available after it (OUT set)
    pub reaching_out: HashMap<usize, HashSet<DefId>>,
}

/// Unique identifier for a definition
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId {
    /// Index into the refs array
    pub ref_index: usize,
    /// Line number of the definition
    pub line: u32,
}

/// Compute reaching definitions for a function
///
/// # Arguments
/// * `cfg` - Control flow graph for the function
/// * `refs` - All variable references in the function
///
/// # Returns
/// * `ReachingDefinitions` - IN and OUT sets for each block
pub fn compute_reaching_definitions(cfg: &CfgInfo, refs: &[VarRef]) -> ReachingDefinitions {
    if cfg.blocks.is_empty() {
        return ReachingDefinitions {
            reaching_in: HashMap::new(),
            reaching_out: HashMap::new(),
        };
    }

    // Build predecessor map from edges
    let mut predecessors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        predecessors.insert(block.id, Vec::new());
    }
    for edge in &cfg.edges {
        predecessors.entry(edge.to).or_default().push(edge.from);
    }

    // Index definitions by line
    let defs: Vec<(DefId, &VarRef)> = refs
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r.ref_type, RefType::Definition | RefType::Update))
        .map(|(i, r)| {
            (
                DefId {
                    ref_index: i,
                    line: r.line,
                },
                r,
            )
        })
        .collect();

    // Compute GEN and KILL sets for each block
    let mut gen: HashMap<usize, HashSet<DefId>> = HashMap::new();
    let mut kill: HashMap<usize, HashSet<DefId>> = HashMap::new();

    // Pre-compute the "owning block" for each definition.
    // When blocks overlap, assign each definition to the LARGEST block
    // that contains it. This prefers actual code blocks over merge points,
    // since merge points tend to be small (single-line) while code blocks
    // span the actual statements.
    // For same-size blocks, prefer HIGHER block ID - in CFG construction,
    // branch bodies (like if-body) typically come after their merge points.
    let def_to_block: HashMap<DefId, usize> = {
        let mut map = HashMap::new();
        for (def_id, var_ref) in &defs {
            let mut best_block: Option<(usize, u32)> = None; // (block_id, size)
            for block in &cfg.blocks {
                let (start, end) = block.lines;
                if var_ref.line >= start && var_ref.line <= end {
                    let size = end - start + 1;
                    // Prefer LARGER blocks (more likely to be actual code blocks)
                    // For same size, prefer HIGHER block ID (branch bodies come after merge points)
                    if best_block.is_none()
                        || size > best_block.unwrap().1
                        || (size == best_block.unwrap().1 && block.id > best_block.unwrap().0)
                    {
                        best_block = Some((block.id, size));
                    }
                }
            }
            if let Some((block_id, _)) = best_block {
                map.insert(*def_id, block_id);
            } else {
                // Orphaned definition (e.g., function parameters on the signature line)
                // falls outside all CFG blocks — assign to entry block
                let entry_id = cfg.blocks.first().map(|b| b.id).unwrap_or(0);
                map.insert(*def_id, entry_id);
            }
        }
        map
    };

    for block in &cfg.blocks {
        let mut block_gen = HashSet::new();
        let mut block_kill = HashSet::new();
        let mut last_def_for_var: HashMap<&str, DefId> = HashMap::new();

        // Find definitions that BELONG to this block (not just within range)
        for (def_id, var_ref) in &defs {
            if def_to_block.get(def_id) == Some(&block.id) {
                // This definition belongs to this block
                // Kill any previous definition of the same variable within this block
                if let Some(&prev) = last_def_for_var.get(var_ref.name.as_str()) {
                    block_kill.insert(prev);
                }
                last_def_for_var.insert(&var_ref.name, *def_id);
            }
        }

        // GEN = last definition of each variable in this block
        block_gen.extend(last_def_for_var.values().copied());

        // KILL = all definitions of variables that are defined in this block
        // (from other blocks - definitions that belong to different blocks)
        let vars_defined: HashSet<&str> = last_def_for_var.keys().copied().collect();
        for (def_id, var_ref) in &defs {
            if vars_defined.contains(var_ref.name.as_str()) {
                // Kill if this definition belongs to a different block
                if def_to_block.get(def_id) != Some(&block.id) {
                    block_kill.insert(*def_id);
                }
            }
        }

        gen.insert(block.id, block_gen);
        kill.insert(block.id, block_kill);
    }

    // Initialize IN and OUT sets
    let mut reaching_in: HashMap<usize, HashSet<DefId>> = HashMap::new();
    let mut reaching_out: HashMap<usize, HashSet<DefId>> = HashMap::new();

    for block in &cfg.blocks {
        reaching_in.insert(block.id, HashSet::new());
        reaching_out.insert(block.id, gen.get(&block.id).cloned().unwrap_or_default());
    }

    // Iterative algorithm until fixed point
    let max_iterations = 100; // Prevent infinite loops
    for _ in 0..max_iterations {
        let mut changed = false;

        for block in &cfg.blocks {
            // IN[B] = union of OUT[P] for all predecessors P
            let mut new_in = HashSet::new();
            if let Some(preds) = predecessors.get(&block.id) {
                for &pred in preds {
                    if let Some(pred_out) = reaching_out.get(&pred) {
                        new_in.extend(pred_out.iter().copied());
                    }
                }
            }

            // OUT[B] = GEN[B] union (IN[B] - KILL[B])
            let block_gen = gen.get(&block.id).cloned().unwrap_or_default();
            let block_kill = kill.get(&block.id).cloned().unwrap_or_default();

            let in_minus_kill: HashSet<DefId> = new_in
                .iter()
                .filter(|d| !block_kill.contains(d))
                .copied()
                .collect();

            let new_out: HashSet<DefId> = block_gen.union(&in_minus_kill).copied().collect();

            // Check if anything changed
            if new_in != *reaching_in.get(&block.id).unwrap_or(&HashSet::new()) {
                changed = true;
                reaching_in.insert(block.id, new_in);
            }
            if new_out != *reaching_out.get(&block.id).unwrap_or(&HashSet::new()) {
                changed = true;
                reaching_out.insert(block.id, new_out);
            }
        }

        if !changed {
            break;
        }
    }

    ReachingDefinitions {
        reaching_in,
        reaching_out,
    }
}

/// Find which definitions reach a specific line
pub fn definitions_reaching_line(
    reaching: &ReachingDefinitions,
    cfg: &CfgInfo,
    refs: &[VarRef],
    line: u32,
    variable: Option<&str>,
) -> Vec<DefId> {
    // Find which block contains this line
    let block_id = cfg
        .blocks
        .iter()
        .find(|b| line >= b.lines.0 && line <= b.lines.1)
        .map(|b| b.id);

    let Some(block_id) = block_id else {
        return Vec::new();
    };

    // Get reaching definitions at this block
    let Some(reaching_in) = reaching.reaching_in.get(&block_id) else {
        return Vec::new();
    };

    // Filter by variable if specified
    let result: Vec<DefId> = reaching_in
        .iter()
        .filter(|def_id| {
            if let Some(var_name) = variable {
                if let Some(var_ref) = refs.get(def_id.ref_index) {
                    var_ref.name == var_name
                } else {
                    false
                }
            } else {
                true
            }
        })
        .copied()
        .collect();

    result
}

// =============================================================================
// Def-Use Chains (RD-7)
// =============================================================================

/// Build def-use chains from reaching definitions analysis.
///
/// For each definition d, finds all uses u where d is in the reaching
/// definitions at u's location and the variable names match.
///
/// # Arguments
/// * `reaching` - Precomputed reaching definitions
/// * `cfg` - Control flow graph
/// * `refs` - All variable references in the function
///
/// # Returns
/// * Vector of `DefUseChain`, one per definition
///
/// # Complexity
/// O(defs * uses) in the worst case, but typically O(defs + uses) when
/// indexed by variable name.
pub fn build_def_use_chains(
    reaching: &ReachingDefinitions,
    cfg: &CfgInfo,
    refs: &[VarRef],
) -> Vec<DefUseChain> {
    // Index definitions by (variable_name, def_id)
    let mut def_chains: HashMap<DefId, DefUseChain> = HashMap::new();

    // Collect all definitions with their metadata
    for (index, var_ref) in refs.iter().enumerate() {
        if matches!(var_ref.ref_type, RefType::Definition | RefType::Update) {
            let def_id = DefId {
                ref_index: index,
                line: var_ref.line,
            };

            // Find which block contains this definition
            let block_id = cfg
                .blocks
                .iter()
                .find(|b| var_ref.line >= b.lines.0 && var_ref.line <= b.lines.1)
                .map(|b| b.id)
                .unwrap_or(0);

            def_chains.insert(
                def_id,
                DefUseChain {
                    definition: Definition {
                        var: var_ref.name.clone(),
                        line: var_ref.line,
                        column: Some(var_ref.column),
                        block: block_id,
                        source_text: None,
                    },
                    uses: Vec::new(),
                },
            );
        }
    }

    // Build an index of definitions by variable name for fast lookup
    // Use String keys to avoid borrowing issues
    let defs_by_var: HashMap<String, Vec<DefId>> = {
        let mut map: HashMap<String, Vec<DefId>> = HashMap::new();
        for (def_id, chain) in &def_chains {
            map.entry(chain.definition.var.clone())
                .or_default()
                .push(*def_id);
        }
        map
    };

    // For each use, find which definitions reach it
    for var_ref in refs.iter() {
        if !matches!(var_ref.ref_type, RefType::Use) {
            continue;
        }

        // Find which block contains this use
        let block_id = cfg
            .blocks
            .iter()
            .find(|b| var_ref.line >= b.lines.0 && var_ref.line <= b.lines.1)
            .map(|b| b.id);

        let Some(block_id) = block_id else {
            continue;
        };

        // Get reaching definitions at this block's entry
        let Some(reaching_in) = reaching.reaching_in.get(&block_id) else {
            continue;
        };

        // Also consider definitions within the same block that appear before this use
        let block_defs_before_use: HashSet<DefId> = refs
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                matches!(r.ref_type, RefType::Definition | RefType::Update)
                    && r.name == var_ref.name
                    && r.line < var_ref.line
            })
            .filter_map(|(i, r)| {
                let in_same_block = cfg
                    .blocks
                    .iter()
                    .find(|b| r.line >= b.lines.0 && r.line <= b.lines.1)
                    .map(|b| b.id)
                    == Some(block_id);
                if in_same_block {
                    Some(DefId {
                        ref_index: i,
                        line: r.line,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Get the variable's candidate definitions
        let Some(var_defs) = defs_by_var.get(&var_ref.name) else {
            continue;
        };

        // Find the last definition in this block before this use (if any)
        let last_local_def = block_defs_before_use.iter().max_by_key(|d| d.line);

        // The use can only see definitions that:
        // 1. Reach the block entry AND are not killed by a local def, OR
        // 2. Are local defs before this use (only the last one)
        let use_site = Use {
            line: var_ref.line,
            column: Some(var_ref.column),
            block: block_id,
            context: var_ref.context.as_ref().map(|c| format!("{:?}", c)),
        };

        for &def_id in var_defs {
            let reaches = if let Some(last_local) = last_local_def {
                // If there's a local def before this use, only that one reaches
                def_id == *last_local
            } else {
                // Otherwise, check if it's in the reaching_in set
                reaching_in.contains(&def_id)
            };

            if reaches {
                if let Some(chain) = def_chains.get_mut(&def_id) {
                    chain.uses.push(use_site.clone());
                }
            }
        }
    }

    def_chains.into_values().collect()
}

// =============================================================================
// Use-Def Chains (RD-8)
// =============================================================================

/// Build use-def chains from reaching definitions analysis.
///
/// For each use u, finds all definitions d where d is in the reaching
/// definitions at u's location and the variable names match.
///
/// This is the inverse of def-use chains.
///
/// # Arguments
/// * `reaching` - Precomputed reaching definitions
/// * `cfg` - Control flow graph
/// * `refs` - All variable references in the function
///
/// # Returns
/// * Vector of `UseDefChain`, one per use
///
/// # Complexity
/// O(uses * defs_per_var) with variable name indexing.
pub fn build_use_def_chains(
    reaching: &ReachingDefinitions,
    cfg: &CfgInfo,
    refs: &[VarRef],
) -> Vec<UseDefChain> {
    let mut chains = Vec::new();

    // Build an index of definitions by variable name
    let defs_by_var: HashMap<&str, Vec<(DefId, &VarRef)>> = {
        let mut map: HashMap<&str, Vec<(DefId, &VarRef)>> = HashMap::new();
        for (index, var_ref) in refs.iter().enumerate() {
            if matches!(var_ref.ref_type, RefType::Definition | RefType::Update) {
                let def_id = DefId {
                    ref_index: index,
                    line: var_ref.line,
                };
                map.entry(var_ref.name.as_str())
                    .or_default()
                    .push((def_id, var_ref));
            }
        }
        map
    };

    // For each use, find all reaching definitions
    for var_ref in refs.iter() {
        if !matches!(var_ref.ref_type, RefType::Use) {
            continue;
        }

        // Find which block contains this use
        let block_id = cfg
            .blocks
            .iter()
            .find(|b| var_ref.line >= b.lines.0 && var_ref.line <= b.lines.1)
            .map(|b| b.id);

        let Some(block_id) = block_id else {
            continue;
        };

        // Get reaching definitions at this block
        let Some(reaching_in) = reaching.reaching_in.get(&block_id) else {
            continue;
        };

        // Get the variable's candidate definitions
        let Some(var_defs) = defs_by_var.get(var_ref.name.as_str()) else {
            // No definitions for this variable - will be flagged as uninitialized
            chains.push(UseDefChain {
                use_site: Use {
                    line: var_ref.line,
                    column: Some(var_ref.column),
                    block: block_id,
                    context: var_ref.context.as_ref().map(|c| format!("{:?}", c)),
                },
                var: var_ref.name.clone(),
                reaching_defs: Vec::new(),
            });
            continue;
        };

        // Find definitions within the same block that appear before this use
        let block_defs_before_use: Vec<&(DefId, &VarRef)> = var_defs
            .iter()
            .filter(|(_, r)| {
                let in_same_block = cfg
                    .blocks
                    .iter()
                    .find(|b| r.line >= b.lines.0 && r.line <= b.lines.1)
                    .map(|b| b.id)
                    == Some(block_id);
                in_same_block && r.line < var_ref.line
            })
            .collect();

        // Find the last definition in this block before this use (if any)
        let last_local_def = block_defs_before_use
            .iter()
            .max_by_key(|(d, _)| d.line)
            .map(|(d, _)| *d);

        let mut reaching_defs = Vec::new();

        for (def_id, def_ref) in var_defs {
            let reaches = if let Some(last_local) = last_local_def {
                // If there's a local def before this use, only that one reaches
                *def_id == last_local
            } else {
                // Otherwise, check if it's in the reaching_in set
                reaching_in.contains(def_id)
            };

            if reaches {
                let def_block_id = cfg
                    .blocks
                    .iter()
                    .find(|b| def_ref.line >= b.lines.0 && def_ref.line <= b.lines.1)
                    .map(|b| b.id)
                    .unwrap_or(0);

                reaching_defs.push(Definition {
                    var: def_ref.name.clone(),
                    line: def_ref.line,
                    column: Some(def_ref.column),
                    block: def_block_id,
                    source_text: None,
                });
            }
        }

        chains.push(UseDefChain {
            use_site: Use {
                line: var_ref.line,
                column: Some(var_ref.column),
                block: block_id,
                context: var_ref.context.as_ref().map(|c| format!("{:?}", c)),
            },
            var: var_ref.name.clone(),
            reaching_defs,
        });
    }

    chains
}

// =============================================================================
// Comprehensive Report Generation
// =============================================================================

/// Generate a comprehensive reaching definitions report.
///
/// Combines reaching definitions analysis with def-use chains, use-def chains,
/// and block-level IN/OUT sets.
///
/// # Arguments
/// * `cfg` - Control flow graph
/// * `refs` - All variable references in the function
/// * `file_path` - Path to the source file
///
/// # Returns
/// * `ReachingDefsReport` - Complete analysis report
pub fn build_reaching_defs_report(
    cfg: &CfgInfo,
    refs: &[VarRef],
    file_path: PathBuf,
) -> ReachingDefsReport {
    build_reaching_defs_report_with_params(cfg, refs, file_path, &[])
}

/// Generate a comprehensive reaching definitions report with explicit parameters.
///
/// Like `build_reaching_defs_report` but allows specifying function parameters
/// explicitly for uninitialized variable detection.
///
/// # Arguments
/// * `cfg` - Control flow graph
/// * `refs` - All variable references in the function
/// * `file_path` - Path to the source file
/// * `params` - Function parameter names (considered pre-initialized)
///
/// # Returns
/// * `ReachingDefsReport` - Complete analysis report
pub fn build_reaching_defs_report_with_params(
    cfg: &CfgInfo,
    refs: &[VarRef],
    file_path: PathBuf,
    params: &[String],
) -> ReachingDefsReport {
    // 1. Compute reaching definitions
    let reaching = compute_reaching_definitions(cfg, refs);

    // Auto-detect parameters from first-line definitions if not provided.
    // Parameters are definitions on the minimum definition line (function signature).
    let detected_params: Vec<String> = if params.is_empty() {
        // Find the minimum line among all definitions - that's the function signature
        let min_def_line = refs
            .iter()
            .filter(|r| matches!(r.ref_type, RefType::Definition))
            .map(|r| r.line)
            .min();

        if let Some(param_line) = min_def_line {
            refs.iter()
                .filter(|r| matches!(r.ref_type, RefType::Definition) && r.line == param_line)
                .map(|r| r.name.clone())
                .collect()
        } else {
            Vec::new()
        }
    } else {
        params.to_vec()
    };

    // 2. Build chains
    let def_use_chains = build_def_use_chains(&reaching, cfg, refs);
    let use_def_chains = build_use_def_chains(&reaching, cfg, refs);

    // 3. Compute block-level IN/OUT sets
    let blocks: Vec<BlockReachingDefs> = cfg
        .blocks
        .iter()
        .map(|b| {
            let in_set = reaching
                .reaching_in
                .get(&b.id)
                .map(|set| def_ids_to_definitions(set, refs, cfg))
                .unwrap_or_default();

            let out_set = reaching
                .reaching_out
                .get(&b.id)
                .map(|set| def_ids_to_definitions(set, refs, cfg))
                .unwrap_or_default();

            // Compute GEN set for this block
            let gen = compute_block_gen(b, refs, cfg);

            // Compute KILL set for this block
            let kill = compute_block_kill(b, refs, cfg);

            BlockReachingDefs {
                id: b.id,
                lines: b.lines,
                gen,
                kill,
                in_set,
                out: out_set,
            }
        })
        .collect();

    // 4. Count definitions and uses
    let def_count = refs
        .iter()
        .filter(|r| matches!(r.ref_type, RefType::Definition | RefType::Update))
        .count();
    let use_count = refs
        .iter()
        .filter(|r| matches!(r.ref_type, RefType::Use))
        .count();

    // 5. Detect uninitialized variables (Phase 9)
    // Use detected parameters to avoid false positives on function parameters
    let uninitialized = detect_uninitialized(&reaching, cfg, refs, &detected_params, &[]);

    // 6. Build stats
    let stats = ReachingDefsStats {
        definitions: def_count,
        uses: use_count,
        blocks: cfg.blocks.len(),
        iterations: 0, // TODO: Track iterations in compute_reaching_definitions
        uninitialized_count: uninitialized.len(),
    };

    // Compute confidence based on analysis completeness
    let total_blocks = cfg.blocks.len();
    let blocks_with_defs = blocks.iter().filter(|b| !b.gen.is_empty()).count();
    let confidence = if total_blocks == 0 {
        crate::dfg::chains::Confidence::Low
    } else if blocks_with_defs > 0 {
        crate::dfg::chains::Confidence::High
    } else {
        crate::dfg::chains::Confidence::Medium
    };

    ReachingDefsReport {
        function: cfg.function.clone(),
        file: file_path,
        blocks,
        def_use_chains,
        use_def_chains,
        uninitialized,
        stats,
        uncertain_defs: Vec::new(),
        confidence,
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Convert a set of DefIds to Definition structs
fn def_ids_to_definitions(
    def_ids: &HashSet<DefId>,
    refs: &[VarRef],
    cfg: &CfgInfo,
) -> Vec<Definition> {
    def_ids
        .iter()
        .filter_map(|def_id| {
            let var_ref = refs.get(def_id.ref_index)?;
            let block_id = cfg
                .blocks
                .iter()
                .find(|b| var_ref.line >= b.lines.0 && var_ref.line <= b.lines.1)
                .map(|b| b.id)
                .unwrap_or(0);

            Some(Definition {
                var: var_ref.name.clone(),
                line: var_ref.line,
                column: Some(var_ref.column),
                block: block_id,
                source_text: None,
            })
        })
        .collect()
}

/// Compute GEN set for a block (last definition of each variable)
fn compute_block_gen(
    block: &crate::types::CfgBlock,
    refs: &[VarRef],
    cfg: &CfgInfo,
) -> Vec<Definition> {
    let (start_line, end_line) = block.lines;
    let is_entry = cfg.blocks.first().map(|b| b.id) == Some(block.id);
    let mut last_def_for_var: HashMap<&str, (usize, &VarRef)> = HashMap::new();

    for (index, var_ref) in refs.iter().enumerate() {
        if matches!(var_ref.ref_type, RefType::Definition | RefType::Update) {
            let in_block = var_ref.line >= start_line && var_ref.line <= end_line;
            // For the entry block, also include orphaned defs (e.g., parameters
            // on the function signature line) that fall before any block
            let is_orphan = is_entry
                && var_ref.line < start_line
                && !cfg
                    .blocks
                    .iter()
                    .any(|b| var_ref.line >= b.lines.0 && var_ref.line <= b.lines.1);
            if in_block || is_orphan {
                last_def_for_var.insert(&var_ref.name, (index, var_ref));
            }
        }
    }

    last_def_for_var
        .values()
        .map(|(_, var_ref)| Definition {
            var: var_ref.name.clone(),
            line: var_ref.line,
            column: Some(var_ref.column),
            block: block.id,
            source_text: None,
        })
        .collect()
}

/// Compute KILL set for a block (definitions of variables that are also defined in this block)
fn compute_block_kill(
    block: &crate::types::CfgBlock,
    refs: &[VarRef],
    cfg: &CfgInfo,
) -> Vec<Definition> {
    let (start_line, end_line) = block.lines;
    let is_entry = cfg.blocks.first().map(|b| b.id) == Some(block.id);

    // Find variables defined in this block (including orphaned defs for entry block)
    let vars_defined_in_block: HashSet<&str> = refs
        .iter()
        .filter(|r| {
            if !matches!(r.ref_type, RefType::Definition | RefType::Update) {
                return false;
            }
            let in_block = r.line >= start_line && r.line <= end_line;
            let is_orphan = is_entry
                && r.line < start_line
                && !cfg
                    .blocks
                    .iter()
                    .any(|b| r.line >= b.lines.0 && r.line <= b.lines.1);
            in_block || is_orphan
        })
        .map(|r| r.name.as_str())
        .collect();

    // Find definitions of those variables outside this block
    refs.iter()
        .filter(|r| {
            if !matches!(r.ref_type, RefType::Definition | RefType::Update) {
                return false;
            }
            if !vars_defined_in_block.contains(r.name.as_str()) {
                return false;
            }
            let in_block = r.line >= start_line && r.line <= end_line;
            let is_orphan = is_entry
                && r.line < start_line
                && !cfg
                    .blocks
                    .iter()
                    .any(|b| r.line >= b.lines.0 && r.line <= b.lines.1);
            !(in_block || is_orphan)
        })
        .map(|var_ref| {
            let def_block_id = cfg
                .blocks
                .iter()
                .find(|b| var_ref.line >= b.lines.0 && var_ref.line <= b.lines.1)
                .map(|b| b.id)
                .unwrap_or(0);

            Definition {
                var: var_ref.name.clone(),
                line: var_ref.line,
                column: Some(var_ref.column),
                block: def_block_id,
                source_text: None,
            }
        })
        .collect()
}

// =============================================================================
// Uninitialized Variable Detection (RD-13) - Phase 9
// =============================================================================

/// Detect potentially uninitialized variable uses.
///
/// A use is considered uninitialized if no definition reaches it on at least one path.
/// The severity is:
/// - `Definite`: No definition exists anywhere in the function
/// - `Possible`: A definition exists but doesn't reach this use on all paths
///
/// # Arguments
/// * `reaching` - Precomputed reaching definitions
/// * `cfg` - Control flow graph
/// * `refs` - All variable references in the function
/// * `params` - Function parameter names (considered pre-initialized)
/// * `globals` - Global variable names (considered pre-initialized)
///
/// # Returns
/// * Vector of `UninitializedUse` for each potentially uninitialized use
///
/// # Example
/// ```text
/// let reaching = compute_reaching_definitions(&cfg, &refs);
/// let uninit = detect_uninitialized(&reaching, &cfg, &refs, &["x".to_string()], &[]);
/// ```
pub fn detect_uninitialized(
    reaching: &ReachingDefinitions,
    cfg: &CfgInfo,
    refs: &[VarRef],
    params: &[String],
    globals: &[String],
) -> Vec<UninitializedUse> {
    let mut uninit = Vec::new();

    // Build set of variables that are "pre-initialized" (params and globals)
    let pre_initialized: HashSet<&str> = params
        .iter()
        .chain(globals.iter())
        .map(|s| s.as_str())
        .collect();

    // Build map of variable -> all definitions (with their DefId)
    let defs_by_var: HashMap<&str, Vec<DefId>> = {
        let mut map: HashMap<&str, Vec<DefId>> = HashMap::new();
        for (index, var_ref) in refs.iter().enumerate() {
            if matches!(var_ref.ref_type, RefType::Definition | RefType::Update) {
                map.entry(var_ref.name.as_str()).or_default().push(DefId {
                    ref_index: index,
                    line: var_ref.line,
                });
            }
        }
        map
    };

    // Build predecessor map from edges
    let mut predecessors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        predecessors.insert(block.id, Vec::new());
    }
    for edge in &cfg.edges {
        predecessors.entry(edge.to).or_default().push(edge.from);
    }

    // Check each use
    for var_ref in refs.iter() {
        if !matches!(var_ref.ref_type, RefType::Use) {
            continue;
        }

        // Skip pre-initialized variables (params and globals)
        if pre_initialized.contains(var_ref.name.as_str()) {
            continue;
        }

        // Find which block contains this use
        let block_id = cfg
            .blocks
            .iter()
            .find(|b| var_ref.line >= b.lines.0 && var_ref.line <= b.lines.1)
            .map(|b| b.id);

        let Some(block_id) = block_id else {
            // Use not in any block - treat as definitely uninitialized
            uninit.push(UninitializedUse {
                var: var_ref.name.clone(),
                line: var_ref.line,
                column: Some(var_ref.column),
                block: 0,
                reason: "use not within any block".to_string(),
                severity: UninitSeverity::Definite,
            });
            continue;
        };

        // Get definitions of this variable
        let var_defs = defs_by_var.get(var_ref.name.as_str());

        if var_defs.is_none() || var_defs.unwrap().is_empty() {
            // No definitions anywhere - definitely uninitialized
            uninit.push(UninitializedUse {
                var: var_ref.name.clone(),
                line: var_ref.line,
                column: Some(var_ref.column),
                block: block_id,
                reason: "no definition of this variable exists".to_string(),
                severity: UninitSeverity::Definite,
            });
            continue;
        }

        let var_defs = var_defs.unwrap();

        // Also check for definitions in the same block before this use
        let local_defs_before_use: Vec<&DefId> = var_defs
            .iter()
            .filter(|d| {
                // Find the ref for this def
                if let Some(def_ref) = refs.get(d.ref_index) {
                    // Check if in same block and before this use
                    let in_same_block = cfg
                        .blocks
                        .iter()
                        .find(|b| def_ref.line >= b.lines.0 && def_ref.line <= b.lines.1)
                        .map(|b| b.id)
                        == Some(block_id);
                    in_same_block && def_ref.line < var_ref.line
                } else {
                    false
                }
            })
            .collect();

        // If there's a local definition before this use, it's initialized
        if !local_defs_before_use.is_empty() {
            continue;
        }

        // Get reaching definitions at this block's entry
        let reaching_in = reaching.reaching_in.get(&block_id);

        // Check if any definition of this variable reaches this block
        let reaching_this_var: Vec<&DefId> = var_defs
            .iter()
            .filter(|d| reaching_in.map(|r| r.contains(d)).unwrap_or(false))
            .collect();

        if reaching_this_var.is_empty() {
            // No definition reaches this use at all
            uninit.push(UninitializedUse {
                var: var_ref.name.clone(),
                line: var_ref.line,
                column: Some(var_ref.column),
                block: block_id,
                reason: "definition exists but does not reach this use on any path".to_string(),
                severity: UninitSeverity::Possible,
            });
            continue;
        }

        // Some definitions reach this use. Check if ALL paths from entry have a definition.
        // This requires tracing back through the CFG to find any path where the variable
        // is not defined.
        //
        // We use a worklist algorithm: starting from the entry block, propagate "no definition"
        // status through blocks that don't generate a definition of this variable.
        // If "no definition" reaches the use block, the variable is possibly uninitialized.
        let has_undefined_path = {
            // Track which blocks have "no definition" reaching them
            let mut no_def_reaches: HashSet<usize> = HashSet::new();
            no_def_reaches.insert(cfg.entry_block);

            // Build successors map
            let mut successors: HashMap<usize, Vec<usize>> = HashMap::new();
            for block in &cfg.blocks {
                successors.insert(block.id, Vec::new());
            }
            for edge in &cfg.edges {
                successors.entry(edge.from).or_default().push(edge.to);
            }

            // Check which blocks generate a definition of this variable.
            // When blocks overlap, assign to LARGEST block (prefer actual code blocks over merge points).
            // For same-size blocks, prefer HIGHER block ID (branch bodies come after merge points).
            let blocks_with_def: HashSet<usize> = var_defs
                .iter()
                .filter_map(|d| {
                    refs.get(d.ref_index).and_then(|r| {
                        let mut best_block: Option<(usize, u32)> = None;
                        for block in &cfg.blocks {
                            let (start, end) = block.lines;
                            if r.line >= start && r.line <= end {
                                let size = end - start + 1;
                                if best_block.is_none()
                                    || size > best_block.unwrap().1
                                    || (size == best_block.unwrap().1
                                        && block.id > best_block.unwrap().0)
                                {
                                    best_block = Some((block.id, size));
                                }
                            }
                        }
                        best_block.map(|(id, _)| id)
                    })
                })
                .collect();

            // Propagate "no definition" through the CFG
            let mut changed = true;
            let mut iterations = 0;
            while changed && iterations < 100 {
                changed = false;
                iterations += 1;

                for block in &cfg.blocks {
                    if no_def_reaches.contains(&block.id) && !blocks_with_def.contains(&block.id) {
                        // This block doesn't define the variable, propagate to successors
                        if let Some(succs) = successors.get(&block.id) {
                            for &succ in succs {
                                if !no_def_reaches.contains(&succ) {
                                    no_def_reaches.insert(succ);
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }

            // Check if "no definition" reaches the use block
            no_def_reaches.contains(&block_id)
        };

        if has_undefined_path {
            // Some paths have definition, some don't - possibly uninitialized
            uninit.push(UninitializedUse {
                var: var_ref.name.clone(),
                line: var_ref.line,
                column: Some(var_ref.column),
                block: block_id,
                reason: "definition does not reach this use on all paths".to_string(),
                severity: UninitSeverity::Possible,
            });
        }
    }

    uninit
}

/// Detect uninitialized variables without parameter/global filtering.
///
/// Convenience function that assumes no parameters or globals.
/// Use `detect_uninitialized` if you need to specify parameters/globals.
pub fn detect_uninitialized_simple(
    reaching: &ReachingDefinitions,
    cfg: &CfgInfo,
    refs: &[VarRef],
) -> Vec<UninitializedUse> {
    detect_uninitialized(reaching, cfg, refs, &[], &[])
}

// =============================================================================
// RPO Worklist Optimization (RD-3) - Phase 16
// =============================================================================

/// Compute reverse postorder of CFG blocks
///
/// RPO ensures that predecessors are processed before successors in acyclic regions,
/// which typically leads to faster convergence of dataflow analysis.
pub fn compute_rpo(cfg: &CfgInfo) -> Vec<usize> {
    let mut visited = HashSet::new();
    let mut postorder = Vec::new();

    fn dfs(
        block_id: usize,
        successors: &HashMap<usize, Vec<usize>>,
        visited: &mut HashSet<usize>,
        postorder: &mut Vec<usize>,
    ) {
        if visited.insert(block_id) {
            if let Some(succs) = successors.get(&block_id) {
                for &succ in succs {
                    dfs(succ, successors, visited, postorder);
                }
            }
            postorder.push(block_id);
        }
    }

    // Build successor map
    let mut successors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        successors.insert(block.id, Vec::new());
    }
    for edge in &cfg.edges {
        successors.entry(edge.from).or_default().push(edge.to);
    }

    dfs(cfg.entry_block, &successors, &mut visited, &mut postorder);
    postorder.reverse();
    postorder
}

/// Result of reaching definitions analysis with iteration statistics
#[derive(Debug, Clone)]
pub struct ReachingDefinitionsWithStats {
    /// Core reaching definitions result
    pub reaching: ReachingDefinitions,
    /// Number of iterations until fixed point
    pub iterations: usize,
}

/// Compute reaching definitions using RPO worklist
///
/// Uses reverse postorder iteration for faster convergence.
/// Typically converges in 2-3 iterations for acyclic CFGs.
///
/// # Arguments
/// * `cfg` - Control flow graph for the function
/// * `refs` - All variable references in the function
///
/// # Returns
/// * `ReachingDefinitionsWithStats` - IN/OUT sets plus iteration count
pub fn compute_reaching_definitions_rpo(
    cfg: &CfgInfo,
    refs: &[VarRef],
) -> ReachingDefinitionsWithStats {
    if cfg.blocks.is_empty() {
        return ReachingDefinitionsWithStats {
            reaching: ReachingDefinitions {
                reaching_in: HashMap::new(),
                reaching_out: HashMap::new(),
            },
            iterations: 0,
        };
    }

    // Build predecessor map from edges
    let mut predecessors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        predecessors.insert(block.id, Vec::new());
    }
    for edge in &cfg.edges {
        predecessors.entry(edge.to).or_default().push(edge.from);
    }

    // Index definitions by line
    let defs: Vec<(DefId, &VarRef)> = refs
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r.ref_type, RefType::Definition | RefType::Update))
        .map(|(i, r)| {
            (
                DefId {
                    ref_index: i,
                    line: r.line,
                },
                r,
            )
        })
        .collect();

    // Compute GEN and KILL sets for each block
    let mut gen: HashMap<usize, HashSet<DefId>> = HashMap::new();
    let mut kill: HashMap<usize, HashSet<DefId>> = HashMap::new();

    for block in &cfg.blocks {
        let (start_line, end_line) = block.lines;
        let mut block_gen = HashSet::new();
        let mut block_kill = HashSet::new();
        let mut last_def_for_var: HashMap<&str, DefId> = HashMap::new();

        // Find definitions in this block
        for (def_id, var_ref) in &defs {
            if var_ref.line >= start_line && var_ref.line <= end_line {
                if let Some(&prev) = last_def_for_var.get(var_ref.name.as_str()) {
                    block_kill.insert(prev);
                }
                last_def_for_var.insert(&var_ref.name, *def_id);
            }
        }

        block_gen.extend(last_def_for_var.values().copied());

        let vars_defined: HashSet<&str> = last_def_for_var.keys().copied().collect();
        for (def_id, var_ref) in &defs {
            if vars_defined.contains(var_ref.name.as_str()) {
                let (start, end) = block.lines;
                if var_ref.line < start || var_ref.line > end {
                    block_kill.insert(*def_id);
                }
            }
        }

        gen.insert(block.id, block_gen);
        kill.insert(block.id, block_kill);
    }

    // Initialize IN and OUT sets
    let mut reaching_in: HashMap<usize, HashSet<DefId>> = HashMap::new();
    let mut reaching_out: HashMap<usize, HashSet<DefId>> = HashMap::new();

    for block in &cfg.blocks {
        reaching_in.insert(block.id, HashSet::new());
        reaching_out.insert(block.id, gen.get(&block.id).cloned().unwrap_or_default());
    }

    // Compute RPO order
    let rpo = compute_rpo(cfg);

    // Iterative algorithm using RPO order until fixed point
    let max_iterations = 100;
    let mut iterations = 0;

    for iteration in 0..max_iterations {
        let mut changed = false;
        iterations = iteration + 1;

        // Process blocks in RPO order
        for &block_id in &rpo {
            // IN[B] = union of OUT[P] for all predecessors P
            let mut new_in = HashSet::new();
            if let Some(preds) = predecessors.get(&block_id) {
                for &pred in preds {
                    if let Some(pred_out) = reaching_out.get(&pred) {
                        new_in.extend(pred_out.iter().copied());
                    }
                }
            }

            // OUT[B] = GEN[B] union (IN[B] - KILL[B])
            let block_gen = gen.get(&block_id).cloned().unwrap_or_default();
            let block_kill = kill.get(&block_id).cloned().unwrap_or_default();

            let in_minus_kill: HashSet<DefId> = new_in
                .iter()
                .filter(|d| !block_kill.contains(d))
                .copied()
                .collect();

            let new_out: HashSet<DefId> = block_gen.union(&in_minus_kill).copied().collect();

            if new_in != *reaching_in.get(&block_id).unwrap_or(&HashSet::new()) {
                changed = true;
                reaching_in.insert(block_id, new_in);
            }
            if new_out != *reaching_out.get(&block_id).unwrap_or(&HashSet::new()) {
                changed = true;
                reaching_out.insert(block_id, new_out);
            }
        }

        if !changed {
            break;
        }
    }

    ReachingDefinitionsWithStats {
        reaching: ReachingDefinitions {
            reaching_in,
            reaching_out,
        },
        iterations,
    }
}

// =============================================================================
// Bit Vector Representation (RD-4) - Phase 16
// =============================================================================

use bitvec::prelude::*;

/// Dense mapping from DefId to bit position for bit vector operations
#[derive(Debug, Clone)]
pub struct DenseDefMapping {
    /// Map definition to bit position
    pub def_to_bit: HashMap<DefId, usize>,
    /// Map bit position to definition
    pub bit_to_def: Vec<DefId>,
    /// Number of definitions
    pub num_defs: usize,
}

/// Create a dense mapping from sparse DefIds to contiguous bit positions
pub fn create_dense_def_mapping(refs: &[VarRef]) -> DenseDefMapping {
    let mut def_to_bit = HashMap::new();
    let mut bit_to_def = Vec::new();

    for (index, var_ref) in refs.iter().enumerate() {
        if matches!(var_ref.ref_type, RefType::Definition | RefType::Update) {
            let def_id = DefId {
                ref_index: index,
                line: var_ref.line,
            };
            def_to_bit.entry(def_id).or_insert_with(|| {
                let idx = bit_to_def.len();
                bit_to_def.push(def_id);
                idx
            });
        }
    }

    DenseDefMapping {
        num_defs: bit_to_def.len(),
        def_to_bit,
        bit_to_def,
    }
}

/// Bit vector-based reaching definitions for efficiency on large definition sets
#[derive(Debug, Clone)]
pub struct BitVectorReachingDefs {
    /// Dense mapping from DefId to bit position
    pub mapping: DenseDefMapping,
    /// IN sets as bit vectors
    pub in_sets: HashMap<usize, BitVec>,
    /// OUT sets as bit vectors
    pub out_sets: HashMap<usize, BitVec>,
    /// Number of iterations to reach fixed point
    pub iterations: usize,
}

impl BitVectorReachingDefs {
    /// Convert to standard ReachingDefinitions format
    pub fn to_standard(&self) -> ReachingDefinitions {
        let mut reaching_in = HashMap::new();
        let mut reaching_out = HashMap::new();

        for (&block_id, in_bv) in &self.in_sets {
            let mut in_set = HashSet::new();
            for (bit_idx, bit) in in_bv.iter().enumerate() {
                if *bit {
                    if let Some(&def_id) = self.mapping.bit_to_def.get(bit_idx) {
                        in_set.insert(def_id);
                    }
                }
            }
            reaching_in.insert(block_id, in_set);
        }

        for (&block_id, out_bv) in &self.out_sets {
            let mut out_set = HashSet::new();
            for (bit_idx, bit) in out_bv.iter().enumerate() {
                if *bit {
                    if let Some(&def_id) = self.mapping.bit_to_def.get(bit_idx) {
                        out_set.insert(def_id);
                    }
                }
            }
            reaching_out.insert(block_id, out_set);
        }

        ReachingDefinitions {
            reaching_in,
            reaching_out,
        }
    }
}

/// Compute reaching definitions using bit vectors for efficiency
///
/// Bit vectors provide O(D/64) set operations where D is the number of definitions,
/// which is more efficient than HashSet for large definition counts.
///
/// # Arguments
/// * `cfg` - Control flow graph for the function
/// * `refs` - All variable references in the function
///
/// # Returns
/// * `BitVectorReachingDefs` - Bit vector based IN/OUT sets
pub fn compute_reaching_definitions_bitvec(
    cfg: &CfgInfo,
    refs: &[VarRef],
) -> BitVectorReachingDefs {
    // Create dense mapping
    let mapping = create_dense_def_mapping(refs);
    let num_defs = mapping.num_defs;

    if cfg.blocks.is_empty() || num_defs == 0 {
        return BitVectorReachingDefs {
            mapping,
            in_sets: HashMap::new(),
            out_sets: HashMap::new(),
            iterations: 0,
        };
    }

    // Build predecessor map from edges
    let mut predecessors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        predecessors.insert(block.id, Vec::new());
    }
    for edge in &cfg.edges {
        predecessors.entry(edge.to).or_default().push(edge.from);
    }

    // Collect all defs with their bit positions
    let defs: Vec<(DefId, &VarRef, usize)> = refs
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r.ref_type, RefType::Definition | RefType::Update))
        .map(|(i, r)| {
            let def_id = DefId {
                ref_index: i,
                line: r.line,
            };
            let bit_pos = *mapping.def_to_bit.get(&def_id).unwrap();
            (def_id, r, bit_pos)
        })
        .collect();

    // Compute GEN and KILL as bit vectors for each block
    let mut gen_bits: HashMap<usize, BitVec> = HashMap::new();
    let mut kill_bits: HashMap<usize, BitVec> = HashMap::new();

    for block in &cfg.blocks {
        let (start_line, end_line) = block.lines;
        let mut block_gen = bitvec![0; num_defs];
        let mut block_kill = bitvec![0; num_defs];
        let mut last_def_for_var: HashMap<&str, usize> = HashMap::new();

        // Find definitions in this block
        for (_def_id, var_ref, bit_pos) in &defs {
            if var_ref.line >= start_line && var_ref.line <= end_line {
                if let Some(&prev_bit) = last_def_for_var.get(var_ref.name.as_str()) {
                    block_kill.set(prev_bit, true);
                }
                last_def_for_var.insert(&var_ref.name, *bit_pos);
            }
        }

        // GEN = last definition of each variable in this block
        for &bit_pos in last_def_for_var.values() {
            block_gen.set(bit_pos, true);
        }

        // KILL = all definitions of variables that are defined in this block (from other blocks)
        let vars_defined: HashSet<&str> = last_def_for_var.keys().copied().collect();
        for (_def_id, var_ref, bit_pos) in &defs {
            if vars_defined.contains(var_ref.name.as_str()) {
                let (start, end) = block.lines;
                if var_ref.line < start || var_ref.line > end {
                    block_kill.set(*bit_pos, true);
                }
            }
        }

        gen_bits.insert(block.id, block_gen);
        kill_bits.insert(block.id, block_kill);
    }

    // Initialize IN and OUT bit vectors
    let mut in_sets: HashMap<usize, BitVec> = HashMap::new();
    let mut out_sets: HashMap<usize, BitVec> = HashMap::new();

    for block in &cfg.blocks {
        in_sets.insert(block.id, bitvec![0; num_defs]);
        out_sets.insert(
            block.id,
            gen_bits
                .get(&block.id)
                .cloned()
                .unwrap_or_else(|| bitvec![0; num_defs]),
        );
    }

    // Compute RPO order for faster convergence
    let rpo = compute_rpo(cfg);

    // Iterative algorithm using bit vector operations
    let max_iterations = 100;
    let mut iterations = 0;

    for iteration in 0..max_iterations {
        let mut changed = false;
        iterations = iteration + 1;

        for &block_id in &rpo {
            // IN[B] = union of OUT[P] for all predecessors P
            let mut new_in = bitvec![0; num_defs];
            if let Some(preds) = predecessors.get(&block_id) {
                for &pred in preds {
                    if let Some(pred_out) = out_sets.get(&pred) {
                        new_in |= pred_out.clone();
                    }
                }
            }

            // OUT[B] = GEN[B] | (IN[B] & !KILL[B])
            let gen = gen_bits
                .get(&block_id)
                .cloned()
                .unwrap_or_else(|| bitvec![0; num_defs]);
            let kill = kill_bits
                .get(&block_id)
                .cloned()
                .unwrap_or_else(|| bitvec![0; num_defs]);

            // Compute IN - KILL = IN & !KILL
            let mut in_minus_kill = new_in.clone();
            in_minus_kill &= !kill;

            // OUT = GEN | (IN - KILL)
            let mut new_out = gen;
            new_out |= in_minus_kill;

            // Check for changes
            if new_in != *in_sets.get(&block_id).unwrap() {
                changed = true;
                in_sets.insert(block_id, new_in);
            }
            if new_out != *out_sets.get(&block_id).unwrap() {
                changed = true;
                out_sets.insert(block_id, new_out);
            }
        }

        if !changed {
            break;
        }
    }

    BitVectorReachingDefs {
        mapping,
        in_sets,
        out_sets,
        iterations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BlockType, CfgBlock, CfgEdge, EdgeType};

    fn make_block(id: usize, start: u32, end: u32) -> CfgBlock {
        CfgBlock {
            id,
            block_type: BlockType::Body,
            lines: (start, end),
            calls: Vec::new(),
        }
    }

    fn make_def(name: &str, line: u32) -> VarRef {
        VarRef {
            name: name.to_string(),
            ref_type: RefType::Definition,
            line,
            column: 0,
            context: None,
            group_id: None,
        }
    }

    fn make_use(name: &str, line: u32) -> VarRef {
        VarRef {
            name: name.to_string(),
            ref_type: RefType::Use,
            line,
            column: 0,
            context: None,
            group_id: None,
        }
    }

    #[test]
    fn test_simple_reaching() {
        // Block 0: x = 1 (line 1)
        // Block 1: y = x (line 2)
        let cfg = CfgInfo {
            function: "test".to_string(),
            blocks: vec![make_block(0, 1, 1), make_block(1, 2, 2)],
            edges: vec![CfgEdge {
                from: 0,
                to: 1,
                edge_type: EdgeType::Unconditional,
                condition: None,
            }],
            entry_block: 0,
            exit_blocks: vec![1],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        };

        let refs = vec![make_def("x", 1), make_use("x", 2)];

        let reaching = compute_reaching_definitions(&cfg, &refs);

        // x's definition should reach block 1
        let block1_in = reaching.reaching_in.get(&1).unwrap();
        assert!(
            block1_in.iter().any(|d| d.ref_index == 0),
            "x's def should reach block 1"
        );
    }

    #[test]
    fn test_kill_set() {
        // Block 0: x = 1 (line 1)
        // Block 1: x = 2 (line 2)
        // Block 2: y = x (line 3) - only x=2 should reach here
        let cfg = CfgInfo {
            function: "test".to_string(),
            blocks: vec![
                make_block(0, 1, 1),
                make_block(1, 2, 2),
                make_block(2, 3, 3),
            ],
            edges: vec![
                CfgEdge {
                    from: 0,
                    to: 1,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 1,
                    to: 2,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
            ],
            entry_block: 0,
            exit_blocks: vec![2],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        };

        let refs = vec![
            make_def("x", 1), // index 0
            make_def("x", 2), // index 1
            make_use("x", 3),
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);

        // Only x=2 (index 1) should reach block 2
        let block2_in = reaching.reaching_in.get(&2).unwrap();
        assert!(
            !block2_in.iter().any(|d| d.ref_index == 0),
            "x=1 should be killed"
        );
        assert!(
            block2_in.iter().any(|d| d.ref_index == 1),
            "x=2 should reach block 2"
        );
    }

    #[test]
    fn test_multiple_paths() {
        // Block 0: entry
        // Block 1: x = 1 (line 1) - true branch
        // Block 2: x = 2 (line 2) - false branch
        // Block 3: y = x (line 3) - both definitions should reach
        let cfg = CfgInfo {
            function: "test".to_string(),
            blocks: vec![
                CfgBlock {
                    id: 0,
                    block_type: BlockType::Entry,
                    lines: (0, 0),
                    calls: Vec::new(),
                },
                make_block(1, 1, 1),
                make_block(2, 2, 2),
                make_block(3, 3, 3),
            ],
            edges: vec![
                CfgEdge {
                    from: 0,
                    to: 1,
                    edge_type: EdgeType::True,
                    condition: None,
                },
                CfgEdge {
                    from: 0,
                    to: 2,
                    edge_type: EdgeType::False,
                    condition: None,
                },
                CfgEdge {
                    from: 1,
                    to: 3,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 2,
                    to: 3,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
            ],
            entry_block: 0,
            exit_blocks: vec![3],
            cyclomatic_complexity: 2,
            nested_functions: HashMap::new(),
        };

        let refs = vec![
            make_def("x", 1), // index 0
            make_def("x", 2), // index 1
            make_use("x", 3),
        ];

        let reaching = compute_reaching_definitions(&cfg, &refs);

        // Both definitions should reach block 3
        let block3_in = reaching.reaching_in.get(&3).unwrap();
        assert!(
            block3_in.iter().any(|d| d.ref_index == 0),
            "x=1 should reach block 3"
        );
        assert!(
            block3_in.iter().any(|d| d.ref_index == 1),
            "x=2 should reach block 3"
        );
    }

    #[test]
    fn test_empty_cfg() {
        let cfg = CfgInfo {
            function: "test".to_string(),
            blocks: Vec::new(),
            edges: Vec::new(),
            entry_block: 0,
            exit_blocks: Vec::new(),
            cyclomatic_complexity: 0,
            nested_functions: HashMap::new(),
        };

        let refs: Vec<VarRef> = Vec::new();
        let reaching = compute_reaching_definitions(&cfg, &refs);

        assert!(reaching.reaching_in.is_empty());
        assert!(reaching.reaching_out.is_empty());
    }
}
