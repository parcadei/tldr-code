//! PDG extraction from source code
//!
//! Builds a Program Dependence Graph by combining CFG and DFG.
//!
//! # PDG Node Types
//! - Entry: Function entry point
//! - Statement: Regular statement
//! - Predicate: Condition in if/while/for
//!
//! # PDG Edge Types
//! - Control: Target is control-dependent on source
//! - Data: Target uses a variable defined by source

use std::collections::HashSet;

use crate::cfg::get_cfg_context;
use crate::dfg::get_dfg_context;
use crate::types::{CfgInfo, DependenceType, DfgInfo, Language, PdgEdge, PdgInfo, PdgNode};
use crate::TldrResult;

/// Extract PDG for a function from source code or file path
///
/// # Arguments
/// * `source_or_path` - Either source code string or path to a file
/// * `function_name` - Name of the function to extract PDG for
/// * `language` - Programming language
///
/// # Returns
/// * `Ok(PdgInfo)` - PDG combining CFG and DFG
/// * `Err(FunctionNotFound)` - If function doesn't exist
pub fn get_pdg_context(
    source_or_path: &str,
    function_name: &str,
    language: Language,
) -> TldrResult<PdgInfo> {
    // Get CFG and DFG
    let cfg = get_cfg_context(source_or_path, function_name, language)?;
    let dfg = get_dfg_context(source_or_path, function_name, language)?;

    // Build PDG from CFG and DFG
    build_pdg(function_name, cfg, dfg)
}

/// Build PDG from CFG and DFG
fn build_pdg(function_name: &str, cfg: CfgInfo, dfg: DfgInfo) -> TldrResult<PdgInfo> {
    let mut nodes: Vec<PdgNode> = Vec::new();
    let mut edges: Vec<PdgEdge> = Vec::new();

    // Create PDG nodes from CFG blocks
    for block in &cfg.blocks {
        // Determine node type from block type
        let node_type = match block.block_type {
            crate::types::BlockType::Entry => "entry".to_string(),
            crate::types::BlockType::Branch => "predicate".to_string(),
            crate::types::BlockType::LoopHeader => "predicate".to_string(),
            _ => "statement".to_string(),
        };

        // Find definitions and uses in this block
        let (defs, uses) = find_defs_uses_in_range(&dfg, block.lines);

        nodes.push(PdgNode {
            id: block.id,
            node_type,
            lines: block.lines,
            definitions: defs,
            uses,
        });
    }

    // Add control dependency edges from CFG
    // A node B is control-dependent on A if:
    // - There's a path from A to B that B post-dominates
    // - A is a predicate (branch/loop)
    for edge in &cfg.edges {
        let from_block = cfg.blocks.iter().find(|b| b.id == edge.from);
        if let Some(block) = from_block {
            // If the source is a branch, add control dependency
            if matches!(
                block.block_type,
                crate::types::BlockType::Branch | crate::types::BlockType::LoopHeader
            ) {
                edges.push(PdgEdge {
                    source_id: edge.from,
                    target_id: edge.to,
                    dep_type: DependenceType::Control,
                    label: format!("control_{:?}", edge.edge_type),
                });
            }
        }
    }

    // Add data dependency edges from DFG
    for dfg_edge in &dfg.edges {
        // Find which blocks contain the def and use
        let def_block = find_block_for_line(&cfg, dfg_edge.def_line);
        let use_block = find_block_for_line(&cfg, dfg_edge.use_line);

        if let (Some(from_id), Some(to_id)) = (def_block, use_block) {
            edges.push(PdgEdge {
                source_id: from_id,
                target_id: to_id,
                dep_type: DependenceType::Data,
                label: dfg_edge.var.clone(),
            });
        }
    }

    Ok(PdgInfo {
        function: function_name.to_string(),
        cfg,
        dfg,
        nodes,
        edges,
    })
}

/// Find definitions and uses in a line range
fn find_defs_uses_in_range(dfg: &DfgInfo, lines: (u32, u32)) -> (Vec<String>, Vec<String>) {
    let (start, end) = lines;
    let mut defs = HashSet::new();
    let mut uses = HashSet::new();

    for r in &dfg.refs {
        if r.line >= start && r.line <= end {
            match r.ref_type {
                crate::types::RefType::Definition | crate::types::RefType::Update => {
                    defs.insert(r.name.clone());
                }
                crate::types::RefType::Use => {
                    uses.insert(r.name.clone());
                }
            }
        }
    }

    (defs.into_iter().collect(), uses.into_iter().collect())
}

/// Find which block contains a given line
fn find_block_for_line(cfg: &CfgInfo, line: u32) -> Option<usize> {
    cfg.blocks
        .iter()
        .find(|b| line >= b.lines.0 && line <= b.lines.1)
        .map(|b| b.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_pdg() {
        let source = r#"
def foo(x):
    y = x + 1
    return y
"#;
        let pdg = get_pdg_context(source, "foo", Language::Python).unwrap();
        assert_eq!(pdg.function, "foo");
        assert!(!pdg.nodes.is_empty());
    }

    #[test]
    fn test_pdg_has_data_dependencies() {
        let source = r#"
def foo():
    x = 1
    y = x + 2
    return y
"#;
        let pdg = get_pdg_context(source, "foo", Language::Python).unwrap();

        // Should have data dependency edges for x
        let data_edges: Vec<_> = pdg
            .edges
            .iter()
            .filter(|e| e.dep_type == DependenceType::Data)
            .collect();

        assert!(!data_edges.is_empty(), "should have data dependency edges");
    }

    #[test]
    fn test_pdg_has_control_dependencies() {
        let source = r#"
def foo(cond):
    if cond:
        x = 1
    else:
        x = 2
    return x
"#;
        let pdg = get_pdg_context(source, "foo", Language::Python).unwrap();

        // Should have control dependency edges from the if condition
        let control_edges: Vec<_> = pdg
            .edges
            .iter()
            .filter(|e| e.dep_type == DependenceType::Control)
            .collect();

        assert!(
            !control_edges.is_empty(),
            "should have control dependency edges"
        );
    }
}
