//! SSA Output Formatting
//!
//! Provides text, JSON, and DOT output formats for SSA.
//!
//! ## Output Formats
//!
//! - **JSON** (SSA-19): Structured output for programmatic consumption
//! - **Text** (SSA-18): LLVM IR-style human-readable output
//! - **DOT** (SSA-20): Graphviz visualization with phi nodes

use super::memory::MemorySsa;
use super::types::*;
use std::collections::HashMap;

/// Format SSA as human-readable text
///
/// # Example Output
/// ```text
/// SSA Form for: process_data in src/main.py
/// Type: minimal
///
/// Block 0 (entry, lines 1-3):
///     x_1 = param
///     y_1 = 0
///
/// Block 1 (lines 4-6):
///     x_2 = phi(x_1 [Block 0], x_3 [Block 2])
///     y_2 = phi(y_1 [Block 0], y_3 [Block 2])
///     t_1 = x_2 + y_2
///
/// ---
/// Phi Functions: 2
/// SSA Names: 8
/// Blocks: 4
/// ```
pub fn format_ssa_text(ssa: &SsaFunction) -> String {
    let mut output = String::new();

    // Build name lookup map (SsaNameId -> "variable_version")
    let name_lookup: HashMap<SsaNameId, String> = ssa
        .ssa_names
        .iter()
        .map(|n| (n.id, n.format_name()))
        .collect();

    // Helper closure to format a name ID
    let fmt_name = |id: SsaNameId| -> String {
        name_lookup
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("${}", id.0))
    };

    // Header
    output.push_str(&format!(
        "SSA Form for: {} in {}\n",
        ssa.function,
        ssa.file.display()
    ));
    output.push_str(&format!("Type: {:?}\n\n", ssa.ssa_type));

    // Blocks
    for block in &ssa.blocks {
        // Block header
        let label = block
            .label
            .as_ref()
            .map(|l| format!("{}, ", l))
            .unwrap_or_default();
        output.push_str(&format!(
            "Block {} ({}lines {}-{}):\n",
            block.id, label, block.lines.0, block.lines.1
        ));

        // Phi functions
        for phi in &block.phi_functions {
            let sources: Vec<String> = phi
                .sources
                .iter()
                .map(|s| format!("{} [Block {}]", fmt_name(s.name), s.block))
                .collect();
            output.push_str(&format!(
                "    {} = phi({})\n",
                fmt_name(phi.target),
                sources.join(", ")
            ));
        }

        // Instructions
        for inst in &block.instructions {
            let target = inst
                .target
                .map(|t| format!("{} = ", fmt_name(t)))
                .unwrap_or_default();
            let uses: Vec<String> = inst.uses.iter().map(|u| fmt_name(*u)).collect();
            let source_text = inst
                .source_text
                .as_ref()
                .map(|s| format!("  // {}", s))
                .unwrap_or_default();

            output.push_str(&format!(
                "    {}{:?}({}){}\n",
                target,
                inst.kind,
                uses.join(", "),
                source_text
            ));
        }

        output.push('\n');
    }

    // Statistics
    output.push_str("---\n");
    output.push_str(&format!("Phi Functions: {}\n", ssa.stats.phi_count));
    output.push_str(&format!("SSA Names: {}\n", ssa.stats.ssa_names));
    output.push_str(&format!("Blocks: {}\n", ssa.stats.blocks));

    if ssa.stats.dead_phi_count > 0 {
        output.push_str(&format!(
            "Dead Phi Functions: {}\n",
            ssa.stats.dead_phi_count
        ));
    }

    output
}

/// Format SSA as DOT graph for visualization
///
/// # Example Output
/// ```dot
/// digraph SSA {
///     rankdir=TB;
///     node [shape=box];
///
///     block0 [label="Block 0 (entry)\n───────────\nx_1 = param\ny_1 = 0"];
///     block1 [label="Block 1\n───────────\nx_2 = phi(x_1, x_3)\nt_1 = x_2 + y_2"];
///
///     block0 -> block1;
///     block1 -> block2 [label="true"];
/// }
/// ```
pub fn format_ssa_dot(ssa: &SsaFunction) -> String {
    let mut output = String::new();

    // Build name lookup map (SsaNameId -> "variable_version")
    let name_lookup: HashMap<SsaNameId, String> = ssa
        .ssa_names
        .iter()
        .map(|n| (n.id, n.format_name()))
        .collect();

    // Helper closure to format a name ID
    let fmt_name = |id: SsaNameId| -> String {
        name_lookup
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("${}", id.0))
    };

    output.push_str("digraph SSA {\n");
    output.push_str("    rankdir=TB;\n");
    output.push_str("    node [shape=box, fontname=\"Courier\"];\n\n");

    // Nodes (blocks)
    for block in &ssa.blocks {
        let label = block
            .label
            .as_ref()
            .map(|l| format!(" ({})", l))
            .unwrap_or_default();

        let mut node_label = format!("Block {}{}", block.id, label);
        node_label.push_str("\\n───────────");

        // Phi functions
        for phi in &block.phi_functions {
            let sources: Vec<String> = phi.sources.iter().map(|s| fmt_name(s.name)).collect();
            node_label.push_str(&format!(
                "\\n{} = phi({})",
                fmt_name(phi.target),
                sources.join(", ")
            ));
        }

        // Instructions (simplified)
        for inst in &block.instructions {
            let target = inst
                .target
                .map(|t| format!("{} = ", fmt_name(t)))
                .unwrap_or_default();
            node_label.push_str(&format!("\\n{}{:?}", target, inst.kind));
        }

        output.push_str(&format!(
            "    block{} [label=\"{}\"];\n",
            block.id, node_label
        ));
    }

    output.push('\n');

    // Edges
    for block in &ssa.blocks {
        for (i, &succ) in block.successors.iter().enumerate() {
            let label = if block.successors.len() > 1 {
                if i == 0 {
                    " [label=\"true\"]"
                } else {
                    " [label=\"false\"]"
                }
            } else {
                ""
            };

            // Check if this is a back edge (successor has lower ID in a loop)
            let style = if succ < block.id {
                " [style=dashed, label=\"back\"]"
            } else {
                label
            };

            output.push_str(&format!(
                "    block{} -> block{}{};\n",
                block.id, succ, style
            ));
        }
    }

    output.push_str("}\n");
    output
}

/// Format Memory SSA as text
pub fn format_memory_ssa_text(memory_ssa: &MemorySsa) -> String {
    let mut output = String::new();

    output.push_str(&format!("Memory SSA for: {}\n\n", memory_ssa.function));

    // Memory Phi nodes
    if !memory_ssa.memory_phis.is_empty() {
        output.push_str("Memory Phi Nodes:\n");
        for phi in &memory_ssa.memory_phis {
            let sources: Vec<String> = phi
                .sources
                .iter()
                .map(|s| format!("{} [Block {}]", s.version, s.block))
                .collect();
            output.push_str(&format!(
                "    Block {}: {} = mem_phi({})\n",
                phi.block,
                phi.result,
                sources.join(", ")
            ));
        }
        output.push('\n');
    }

    // Memory Definitions
    if !memory_ssa.memory_defs.is_empty() {
        output.push_str("Memory Definitions (Stores):\n");
        for def in &memory_ssa.memory_defs {
            output.push_str(&format!(
                "    Line {}: {} = store({}) [clobbers {}]\n",
                def.line, def.version, def.access, def.clobbers
            ));
        }
        output.push('\n');
    }

    // Memory Uses
    if !memory_ssa.memory_uses.is_empty() {
        output.push_str("Memory Uses (Loads):\n");
        for use_ in &memory_ssa.memory_uses {
            output.push_str(&format!(
                "    Line {}: load({}) [uses {}]\n",
                use_.line, use_.access, use_.version
            ));
        }
    }

    output
}

/// Validate DOT output is syntactically correct
pub fn validate_dot(dot: &str) -> bool {
    // Basic validation: must start with digraph and end with }
    let trimmed = dot.trim();
    trimmed.starts_with("digraph") && trimmed.ends_with('}')
}

/// Validate JSON output can be parsed back
pub fn validate_json(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json).is_ok()
}

// =============================================================================
// JSON Output Functions (SSA-19)
// =============================================================================

/// Format SSA function as JSON (pretty-printed)
///
/// # Example
/// ```rust,ignore
/// let ssa = construct_minimal_ssa(&cfg, &dfg)?;
/// let json = format_ssa_json(&ssa)?;
/// println!("{}", json);
/// ```
pub fn format_ssa_json(ssa: &SsaFunction) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(ssa)
}

/// Format SSA function as compact JSON (no pretty printing)
///
/// Useful for serialization where size matters more than readability.
pub fn format_ssa_json_compact(ssa: &SsaFunction) -> Result<String, serde_json::Error> {
    serde_json::to_string(ssa)
}

// =============================================================================
// Enhanced DOT Output
// =============================================================================

/// Format SSA as DOT graph with def-use edges shown
///
/// This version includes dashed blue edges showing def-use relationships
/// in addition to control flow edges.
pub fn format_ssa_dot_with_def_use(ssa: &SsaFunction) -> String {
    let mut output = String::new();

    output.push_str("digraph SSA {\n");
    output.push_str("    rankdir=TB;\n");
    output.push_str("    node [shape=box, fontname=\"Courier\"];\n\n");

    // Build a map from SsaNameId to defining block for def-use edges
    let mut def_block_map: HashMap<SsaNameId, usize> = HashMap::new();
    for name in &ssa.ssa_names {
        if let Some(block) = name.def_block {
            def_block_map.insert(name.id, block);
        }
    }

    // Also map phi targets to their blocks
    for block in &ssa.blocks {
        for phi in &block.phi_functions {
            def_block_map.insert(phi.target, block.id);
        }
    }

    // Nodes (blocks)
    for block in &ssa.blocks {
        let label = block
            .label
            .as_ref()
            .map(|l| format!(" ({})", escape_dot_label(l)))
            .unwrap_or_default();

        let mut node_label = format!("Block {}{}", block.id, label);
        node_label.push_str("\\n───────────");

        // Phi functions
        for phi in &block.phi_functions {
            let sources: Vec<String> = phi.sources.iter().map(|s| s.name.to_string()).collect();
            node_label.push_str(&format!("\\n{} = phi({})", phi.target, sources.join(", ")));
        }

        // Instructions (simplified)
        for inst in &block.instructions {
            let target = inst.target.map(|t| format!("{} = ", t)).unwrap_or_default();
            node_label.push_str(&format!("\\n{}{:?}", target, inst.kind));
        }

        output.push_str(&format!(
            "    block{} [label=\"{}\"];\n",
            block.id, node_label
        ));
    }

    output.push('\n');

    // Control flow edges
    output.push_str("    // Control flow edges\n");
    for block in &ssa.blocks {
        for (i, &succ) in block.successors.iter().enumerate() {
            let label = if block.successors.len() > 1 {
                if i == 0 {
                    " [label=\"true\"]"
                } else {
                    " [label=\"false\"]"
                }
            } else {
                ""
            };

            // Check if this is a back edge (successor has lower ID in a loop)
            let style = if succ < block.id {
                " [style=dashed, label=\"back\"]"
            } else {
                label
            };

            output.push_str(&format!(
                "    block{} -> block{}{};\n",
                block.id, succ, style
            ));
        }
    }

    // Def-use edges
    output.push_str("\n    // Def-use chains\n");
    output.push_str("    edge [style=dashed, color=blue, constraint=false];\n");

    for (def_id, uses) in &ssa.def_use {
        if let Some(&def_block) = def_block_map.get(def_id) {
            for use_id in uses {
                // Find the block where this use occurs
                for block in &ssa.blocks {
                    let used_in_block = block
                        .instructions
                        .iter()
                        .any(|inst| inst.uses.contains(use_id))
                        || block
                            .phi_functions
                            .iter()
                            .any(|phi| phi.sources.iter().any(|s| &s.name == use_id));

                    if used_in_block && def_block != block.id {
                        // Get the variable name for labeling
                        let var_name = ssa
                            .ssa_names
                            .iter()
                            .find(|n| n.id == *def_id)
                            .map(|n| n.format_name())
                            .unwrap_or_else(|| def_id.to_string());

                        output.push_str(&format!(
                            "    block{} -> block{} [label=\"{}\"];\n",
                            def_block, block.id, var_name
                        ));
                        break;
                    }
                }
            }
        }
    }

    output.push_str("}\n");
    output
}

/// Escape special characters for DOT label strings
fn escape_dot_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('<', "\\<")
        .replace('>', "\\>")
        .replace('|', "\\|")
        .replace('{', "\\{")
        .replace('}', "\\}")
        .replace('\n', "\\n")
}
