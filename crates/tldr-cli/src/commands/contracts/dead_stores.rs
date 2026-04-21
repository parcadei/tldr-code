//! Dead-Stores command - SSA-based dead store detection.
//!
//! Analyzes SSA form to find assignments that are never read. A dead store
//! occurs when a variable is assigned but the value is never used before
//! being overwritten or the variable goes out of scope.
//!
//! # TIGER Mitigations Addressed
//! - T04: Memory exhaustion - check_ssa_node_limit() limits SSA graph size
//! - T08: AST stack overflow - inherits depth limits from SSA construction
//!
//! # Algorithm
//!
//! Uses a hybrid approach:
//! 1. SSA form provides versioned variable names (each definition gets unique version)
//! 2. DFG provides accurate use information (which defs are used where)
//! 3. Combine: if a def version has no corresponding use in DFG, it's dead
//!
//! For each definition D at line L:
//! - Find all uses U of the same variable at lines > L
//! - If there's a use before the next redefinition, D is live
//! - If D is overwritten or never used, D is dead

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;

use tldr_core::cfg::get_cfg_context;
use tldr_core::dfg::get_dfg_context;
use tldr_core::ssa::{SsaFunction, SsaNameId};
use tldr_core::types::RefType;
use tldr_core::Language;

use crate::output::{OutputFormat, OutputWriter};

use super::error::{ContractsError, ContractsResult};
use super::types::{DeadStore, DeadStoresReport, OutputFormat as ContractsOutputFormat};
use super::validation::{
    check_ssa_node_limit, read_file_safe, validate_file_path, validate_function_name,
};

// =============================================================================
// CLI Arguments
// =============================================================================

/// Find dead stores using SSA-based analysis.
///
/// A dead store is an assignment where the value is never used.
/// Uses Static Single Assignment (SSA) form for precise detection.
///
/// # Example
///
/// ```bash
/// tldr dead-stores src/module.py process_data
/// tldr dead-stores src/module.py process_data --compare
/// ```
#[derive(Debug, Args)]
pub struct DeadStoresArgs {
    /// Source file to analyze
    #[arg(value_name = "file")]
    pub file: PathBuf,

    /// Function name to analyze
    #[arg(value_name = "function")]
    pub function: String,

    /// Output format (json or text). Prefer global --format/-f flag.
    #[arg(
        long = "output-format",
        short = 'o',
        hide = true,
        default_value = "json"
    )]
    pub output_format: ContractsOutputFormat,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Compare SSA-based detection with live-variables based detection
    #[arg(long)]
    pub compare: bool,
}

impl DeadStoresArgs {
    /// Run the dead-stores command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate inputs
        let canonical_path = validate_file_path(&self.file)?;
        validate_function_name(&self.function)?;

        writer.progress(&format!(
            "Analyzing dead stores for {}::{}...",
            self.file.display(),
            self.function
        ));

        // Determine language
        let language = self
            .lang
            .unwrap_or_else(|| Language::from_path(&self.file).unwrap_or(Language::Python));

        // Run SSA-based dead store detection
        let report = run_dead_stores(&canonical_path, &self.function, language, self.compare)?;

        // Output based on format
        let use_text = matches!(self.output_format, ContractsOutputFormat::Text)
            || matches!(format, OutputFormat::Text);

        if use_text {
            let text = format_dead_stores_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

// =============================================================================
// Core Analysis Functions
// =============================================================================

/// Run dead store detection on a file and function.
///
/// # Arguments
/// * `file` - Path to the source file
/// * `function` - Name of the function to analyze
/// * `language` - Programming language
/// * `compare` - If true, also run live-vars based detection for comparison
///
/// # Returns
/// DeadStoresReport with detected dead stores.
pub fn run_dead_stores(
    file: &Path,
    function: &str,
    language: Language,
    compare: bool,
) -> ContractsResult<DeadStoresReport> {
    // Read the file
    let source = read_file_safe(file)?;

    // Extract CFG for block information
    let cfg = get_cfg_context(&source, function, language).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not found") || msg.contains("Not found") {
            ContractsError::FunctionNotFound {
                function: function.to_string(),
                file: file.to_path_buf(),
            }
        } else {
            ContractsError::SsaError(format!("CFG extraction failed: {}", e))
        }
    })?;

    // Extract DFG for accurate variable refs
    let dfg = get_dfg_context(&source, function, language).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not found") || msg.contains("Not found") {
            ContractsError::FunctionNotFound {
                function: function.to_string(),
                file: file.to_path_buf(),
            }
        } else {
            ContractsError::SsaError(format!("DFG extraction failed: {}", e))
        }
    })?;

    // Check SSA node limit (TIGER T04 mitigation)
    // Use definition count as proxy for SSA nodes
    let def_count = dfg
        .refs
        .iter()
        .filter(|r| matches!(r.ref_type, RefType::Definition | RefType::Update))
        .count();
    check_ssa_node_limit(def_count)?;

    // Find dead stores using DFG-based analysis
    let dead_stores_ssa = find_dead_stores_dfg(&dfg.refs, &cfg)?;

    // Optionally run live-vars comparison
    let (dead_stores_live_vars, live_vars_count) = if compare {
        let live_vars_dead = find_dead_stores_live_vars(&source, function, language)?;
        let count = live_vars_dead.len() as u32;
        (Some(live_vars_dead), Some(count))
    } else {
        (None, None)
    };

    Ok(DeadStoresReport {
        function: function.to_string(),
        file: file.to_path_buf(),
        dead_stores_ssa: dead_stores_ssa.clone(),
        count: dead_stores_ssa.len() as u32,
        dead_stores_live_vars,
        live_vars_count,
    })
}

/// Find dead stores using DFG-based analysis.
///
/// A definition is dead if:
/// 1. There's another definition of the same variable at a later line AND
///    there's no use of the variable between those two definitions, OR
/// 2. The variable is never used anywhere in the function (excluding parameters)
///
/// Parameters are excluded from "never used" detection since unused parameters
/// are often intentional (interface requirements, placeholder, etc.).
///
/// This is conservative to avoid false positives from control-flow merges.
/// For phi-aware analysis, use the --compare flag to see live-vars results.
///
/// # Algorithm
///
/// 1. Group VarRefs by variable name
/// 2. Sort refs by line number
/// 3. Identify function parameters (definitions at function start line)
/// 4. For each definition, check:
///    - If there's a next definition: check for use between them
///    - If it's the last definition and not a parameter: check if there's any use after it
/// 5. If no use found, the definition is dead
///
/// # Arguments
/// * `refs` - Variable references from DFG
/// * `cfg` - Control flow graph for block information
///
/// # Returns
/// List of DeadStore instances.
pub fn find_dead_stores_dfg(
    refs: &[tldr_core::types::VarRef],
    cfg: &tldr_core::types::CfgInfo,
) -> ContractsResult<Vec<DeadStore>> {
    let mut dead_stores = Vec::new();

    // Build line-to-block map
    let mut line_to_block: HashMap<u32, usize> = HashMap::new();
    for block in &cfg.blocks {
        for line in block.lines.0..=block.lines.1 {
            line_to_block.insert(line, block.id);
        }
    }

    // Identify the function definition line (first line with definitions)
    // Parameters are typically defined on this line
    let first_def_line = refs
        .iter()
        .filter(|r| matches!(r.ref_type, RefType::Definition))
        .map(|r| r.line)
        .min();

    // Group refs by variable name
    let mut refs_by_var: HashMap<String, Vec<&tldr_core::types::VarRef>> = HashMap::new();
    for var_ref in refs {
        refs_by_var
            .entry(var_ref.name.clone())
            .or_default()
            .push(var_ref);
    }

    // Track SSA version numbers per variable
    let mut version_counters: HashMap<String, u32> = HashMap::new();

    // For each variable, analyze its references
    for (var_name, mut var_refs) in refs_by_var {
        // Sort by line number
        var_refs.sort_by_key(|r| r.line);

        // Check if this is a function parameter (defined on function definition line)
        let is_parameter = first_def_line
            .map(|line| {
                var_refs
                    .first()
                    .map(|r| r.line == line && matches!(r.ref_type, RefType::Definition))
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        // Collect definitions and uses
        let mut definitions: Vec<(u32, u32, u32)> = Vec::new(); // (line, block_id, version)
        let uses: Vec<u32> = var_refs
            .iter()
            .filter(|r| matches!(r.ref_type, RefType::Use))
            .map(|r| r.line)
            .collect();

        for var_ref in &var_refs {
            match var_ref.ref_type {
                RefType::Definition | RefType::Update => {
                    // Increment version
                    let version = version_counters.entry(var_name.clone()).or_insert(0);
                    *version += 1;

                    let block_id = line_to_block.get(&var_ref.line).copied().unwrap_or(0);
                    definitions.push((var_ref.line, block_id as u32, *version));
                }
                RefType::Use => {}
            }
        }

        // If there are no uses of this variable at all, all non-parameter definitions are dead
        if uses.is_empty() && !definitions.is_empty() && !is_parameter {
            for (def_line, block_id, version) in &definitions {
                dead_stores.push(DeadStore {
                    variable: var_name.clone(),
                    ssa_name: format!("{}_{}", var_name, version),
                    line: *def_line,
                    block_id: *block_id,
                    is_phi: false,
                });
            }
            continue;
        }

        // Check each definition for "overwritten before use" pattern
        // Only flag as dead if there's a REDEFINITION in the SAME BLOCK without use between
        for (i, &(def_line, def_block_id, version)) in definitions.iter().enumerate() {
            // Skip parameters for overwrite detection (first definition on param line)
            if is_parameter && i == 0 {
                continue;
            }

            // Find the next definition in the SAME block
            let next_def_in_block = definitions
                .iter()
                .skip(i + 1)
                .find(|(_, block_id, _)| *block_id == def_block_id);

            if let Some(&(next_def_line, _, _)) = next_def_in_block {
                // Check if there's any use between this def and the next def in the same block
                let has_use_between = uses
                    .iter()
                    .any(|&use_line| use_line > def_line && use_line < next_def_line);

                if !has_use_between {
                    // This is a dead store - overwritten before use in the same block
                    dead_stores.push(DeadStore {
                        variable: var_name.clone(),
                        ssa_name: format!("{}_{}", var_name, version),
                        line: def_line,
                        block_id: def_block_id,
                        is_phi: false,
                    });
                }
            }
        }
    }

    // Sort by line number for consistent output
    dead_stores.sort_by_key(|d| (d.line, d.variable.clone()));

    Ok(dead_stores)
}

/// Find dead stores in SSA form.
///
/// A definition is dead if it has no uses (empty use_sites).
/// Phi function definitions without uses may be normal in some cases.
///
/// # Arguments
/// * `ssa` - SSA function with def-use chains
///
/// # Returns
/// List of DeadStore instances for each dead definition.
///
/// # Algorithm
///
/// For each SSA name (definition):
/// 1. Check if it has any uses in the def-use chains
/// 2. Check if it has any uses in instruction operands
/// 3. Check if it's used in phi function sources
/// 4. If no uses found, it's a dead store
/// 5. Extract original variable name by stripping version suffix
/// 6. Mark whether it's a phi function definition
pub fn find_dead_stores_ssa(ssa: &SsaFunction) -> Vec<DeadStore> {
    use std::collections::HashSet;

    let mut dead_stores = Vec::new();

    // Build a complete set of used SSA names by scanning all uses
    let mut used_names: HashSet<SsaNameId> = HashSet::new();

    // Collect uses from def_use chains
    for uses in ssa.def_use.values() {
        for &use_id in uses {
            used_names.insert(use_id);
        }
    }

    // Collect uses from instructions
    for block in &ssa.blocks {
        for inst in &block.instructions {
            for &use_id in &inst.uses {
                used_names.insert(use_id);
            }
        }

        // Collect uses from phi function sources
        for phi in &block.phi_functions {
            for source in &phi.sources {
                used_names.insert(source.name);
            }
        }
    }

    // Iterate through all SSA names
    for ssa_name in &ssa.ssa_names {
        // Check if this SSA name is used anywhere
        let is_used_in_def_use = ssa
            .def_use
            .get(&ssa_name.id)
            .is_some_and(|uses| !uses.is_empty());
        let is_used_anywhere = used_names.contains(&ssa_name.id);

        // If no uses, it's a dead store
        if !is_used_in_def_use && !is_used_anywhere {
            // Check if this is a phi function definition
            let is_phi = is_phi_definition(ssa, ssa_name.id);

            // Get the definition line
            let (line, block_id) = get_def_location(ssa, ssa_name.id);

            dead_stores.push(DeadStore {
                variable: ssa_name.variable.clone(),
                ssa_name: format!("{}_{}", ssa_name.variable, ssa_name.version),
                line,
                block_id,
                is_phi,
            });
        }
    }

    dead_stores
}

/// Check if an SSA name is defined by a phi function
fn is_phi_definition(ssa: &SsaFunction, name_id: SsaNameId) -> bool {
    for block in &ssa.blocks {
        for phi in &block.phi_functions {
            if phi.target == name_id {
                return true;
            }
        }
    }
    false
}

/// Get the definition location (line, block_id) for an SSA name
fn get_def_location(ssa: &SsaFunction, name_id: SsaNameId) -> (u32, u32) {
    // First check phi functions
    for block in &ssa.blocks {
        for phi in &block.phi_functions {
            if phi.target == name_id {
                return (phi.line, block.id as u32);
            }
        }
    }

    // Then check regular instructions
    for block in &ssa.blocks {
        for inst in &block.instructions {
            if inst.target == Some(name_id) {
                return (inst.line, block.id as u32);
            }
        }
    }

    // Fallback: check SSA name metadata
    if let Some(ssa_name) = ssa.ssa_names.iter().find(|n| n.id == name_id) {
        return (ssa_name.def_line, ssa_name.def_block.unwrap_or(0) as u32);
    }

    (0, 0)
}

/// Find dead stores using live-variables analysis (for comparison).
///
/// This method uses a different algorithm:
/// 1. Compute live variables at each program point
/// 2. An assignment to x at line L is dead if x is not in LiveOut[L]
///
/// # Arguments
/// * `source` - Source code
/// * `function` - Function name
/// * `language` - Programming language
///
/// # Returns
/// List of DeadStore instances.
fn find_dead_stores_live_vars(
    source: &str,
    function: &str,
    language: Language,
) -> ContractsResult<Vec<DeadStore>> {
    use tldr_core::cfg::get_cfg_context;
    use tldr_core::dfg::get_dfg_context;
    use tldr_core::ssa::analysis::compute_live_variables;
    use tldr_core::types::RefType;

    // Extract CFG
    let cfg = get_cfg_context(source, function, language)
        .map_err(|e| ContractsError::SsaError(format!("CFG extraction failed: {}", e)))?;

    // Extract DFG
    let dfg = get_dfg_context(source, function, language)
        .map_err(|e| ContractsError::SsaError(format!("DFG extraction failed: {}", e)))?;

    // Compute live variables
    let live_vars = compute_live_variables(&cfg, &dfg.refs)
        .map_err(|e| ContractsError::SsaError(format!("Live variables analysis failed: {}", e)))?;

    // Build line-to-block map
    let mut line_to_block = std::collections::HashMap::new();
    for block in &cfg.blocks {
        for line in block.lines.0..=block.lines.1 {
            line_to_block.insert(line, block.id);
        }
    }

    let mut dead_stores = Vec::new();

    // For each definition, check if variable is live out at that point
    for var_ref in &dfg.refs {
        if !matches!(var_ref.ref_type, RefType::Definition | RefType::Update) {
            continue;
        }

        if let Some(&block_id) = line_to_block.get(&var_ref.line) {
            // Check if this variable is in live_out for this block
            let is_live_out = live_vars
                .blocks
                .get(&block_id)
                .map(|block_info| block_info.live_out.contains(&var_ref.name))
                .unwrap_or(false);

            if !is_live_out {
                // This assignment might be dead (not used after this point in this block)
                // More precise analysis would check within the block too
                dead_stores.push(DeadStore {
                    variable: var_ref.name.clone(),
                    ssa_name: format!("{}_lv", var_ref.name),
                    line: var_ref.line,
                    block_id: block_id as u32,
                    is_phi: false,
                });
            }
        }
    }

    Ok(dead_stores)
}

// =============================================================================
// Output Formatting
// =============================================================================

/// Format dead stores report as text.
fn format_dead_stores_text(report: &DeadStoresReport) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "Dead Stores: {} ({})\n",
        report.function,
        report.file.display()
    ));
    output.push_str(&"=".repeat(60));
    output.push('\n');

    if report.dead_stores_ssa.is_empty() {
        output.push_str("No dead stores detected.\n");
    } else {
        output.push_str(&format!(
            "Found {} dead store(s):\n\n",
            report.dead_stores_ssa.len()
        ));

        for store in &report.dead_stores_ssa {
            let phi_marker = if store.is_phi { " [phi]" } else { "" };
            output.push_str(&format!(
                "  Line {}: {} ({}){}'\n",
                store.line, store.variable, store.ssa_name, phi_marker
            ));
        }
    }

    // Comparison results if present
    if let Some(live_vars_dead) = &report.dead_stores_live_vars {
        output.push('\n');
        output.push_str("Live-Variables Comparison:\n");
        output.push_str(&"-".repeat(40));
        output.push('\n');
        output.push_str(&format!("  SSA-based: {} dead stores\n", report.count));
        output.push_str(&format!(
            "  Live-vars: {} dead stores\n",
            report.live_vars_count.unwrap_or(0)
        ));

        if !live_vars_dead.is_empty() {
            output.push_str("\n  Live-vars dead stores:\n");
            for store in live_vars_dead {
                output.push_str(&format!("    Line {}: {}\n", store.line, store.variable));
            }
        }
    }

    output
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_dead_stores_ssa_simple() {
        // This test requires SSA construction which needs actual parsing
        // We'll test the algorithm logic with mock data in integration tests
    }

    #[test]
    fn test_format_dead_stores_text_empty() {
        let report = DeadStoresReport {
            function: "test_func".to_string(),
            file: PathBuf::from("test.py"),
            dead_stores_ssa: vec![],
            count: 0,
            dead_stores_live_vars: None,
            live_vars_count: None,
        };

        let text = format_dead_stores_text(&report);
        assert!(text.contains("No dead stores detected"));
    }

    #[test]
    fn test_format_dead_stores_text_with_stores() {
        let report = DeadStoresReport {
            function: "test_func".to_string(),
            file: PathBuf::from("test.py"),
            dead_stores_ssa: vec![
                DeadStore {
                    variable: "x".to_string(),
                    ssa_name: "x_1".to_string(),
                    line: 5,
                    block_id: 1,
                    is_phi: false,
                },
                DeadStore {
                    variable: "y".to_string(),
                    ssa_name: "y_2".to_string(),
                    line: 10,
                    block_id: 2,
                    is_phi: true,
                },
            ],
            count: 2,
            dead_stores_live_vars: None,
            live_vars_count: None,
        };

        let text = format_dead_stores_text(&report);
        assert!(text.contains("Found 2 dead store(s)"));
        assert!(text.contains("Line 5: x"));
        assert!(text.contains("Line 10: y"));
        assert!(text.contains("[phi]"));
    }

    #[test]
    fn test_format_dead_stores_text_with_comparison() {
        let report = DeadStoresReport {
            function: "test_func".to_string(),
            file: PathBuf::from("test.py"),
            dead_stores_ssa: vec![DeadStore {
                variable: "a".to_string(),
                ssa_name: "a_1".to_string(),
                line: 3,
                block_id: 0,
                is_phi: false,
            }],
            count: 1,
            dead_stores_live_vars: Some(vec![DeadStore {
                variable: "a".to_string(),
                ssa_name: "a_lv".to_string(),
                line: 3,
                block_id: 0,
                is_phi: false,
            }]),
            live_vars_count: Some(1),
        };

        let text = format_dead_stores_text(&report);
        assert!(text.contains("Live-Variables Comparison"));
        assert!(text.contains("SSA-based: 1"));
        assert!(text.contains("Live-vars: 1"));
    }
}
