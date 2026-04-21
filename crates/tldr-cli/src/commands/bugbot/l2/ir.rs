//! Shared IR (Intermediate Representation) construction helpers.
//!
//! These functions build per-function analysis artifacts (CFG, DFG, SSA,
//! taint, abstract interpretation) that the FlowEngine calls during deep
//! analysis. Each function delegates to the corresponding `tldr_core` API.
//!
//! # Architecture
//!
//! All builders accept the full file source and the `FunctionId` (which
//! carries the function's qualified name). They call into `tldr_core`'s
//! public extraction APIs:
//!
//! - CFG: `tldr_core::get_cfg_context`
//! - DFG: `tldr_core::get_dfg_context`
//! - SSA: `tldr_core::ssa::construct_ssa`
//! - Taint: `tldr_core::compute_taint_with_tree`
//! - Abstract interp: `tldr_core::compute_abstract_interp`

use std::collections::HashMap;

use anyhow::{Context, Result};

use tldr_core::Language;

use super::types::FunctionId;

/// Build a CFG (Control Flow Graph) for a specific function.
///
/// Calls `tldr_core::get_cfg_context` with the file source and function name.
/// Returns an empty CFG if the function is not found (per the tldr_core spec).
pub fn build_cfg_for_function(
    file_contents: &str,
    function_id: &FunctionId,
    language: Language,
) -> Result<tldr_core::CfgInfo> {
    tldr_core::get_cfg_context(file_contents, &function_id.qualified_name, language)
        .map_err(|e| anyhow::anyhow!(e))
        .with_context(|| format!("CFG construction for {}", function_id))
}

/// Build a DFG (Data Flow Graph) for a specific function.
///
/// Calls `tldr_core::get_dfg_context` with the file source and function name.
/// Fails if the function is not found (returns FunctionNotFound error).
pub fn build_dfg_for_function(
    file_contents: &str,
    function_id: &FunctionId,
    language: Language,
) -> Result<tldr_core::DfgInfo> {
    tldr_core::get_dfg_context(file_contents, &function_id.qualified_name, language)
        .map_err(|e| anyhow::anyhow!(e))
        .with_context(|| format!("DFG construction for {}", function_id))
}

/// Build SSA form for a specific function.
///
/// Calls `tldr_core::ssa::construct_ssa` which internally builds CFG and DFG
/// before constructing SSA. Uses `SsaType::Minimal` for speed.
pub fn build_ssa_for_function(
    file_contents: &str,
    function_id: &FunctionId,
    language: Language,
) -> Result<tldr_core::ssa::SsaFunction> {
    tldr_core::ssa::construct_ssa(
        file_contents,
        &function_id.qualified_name,
        language,
        tldr_core::ssa::SsaType::Minimal,
    )
    .map_err(|e| anyhow::anyhow!(e))
    .with_context(|| format!("SSA construction for {}", function_id))
}

/// Run taint analysis for a specific function.
///
/// Builds the CFG and DFG, then runs taint propagation using
/// `tldr_core::compute_taint_with_tree`. Uses tree-sitter AST for enhanced
/// source/sink detection when possible.
pub fn build_taint_for_function(
    file_contents: &str,
    function_id: &FunctionId,
    language: Language,
) -> Result<tldr_core::TaintInfo> {
    // Build CFG and DFG first
    let cfg = build_cfg_for_function(file_contents, function_id, language)?;
    let dfg = build_dfg_for_function(file_contents, function_id, language)?;

    // Build statement map: line number -> line text
    let statements: HashMap<u32, String> = file_contents
        .lines()
        .enumerate()
        .map(|(i, line)| ((i + 1) as u32, line.to_string()))
        .collect();

    // Parse tree for AST-enhanced taint detection
    let tree = tldr_core::ast::parser::parse(file_contents, language)
        .map_err(|e| anyhow::anyhow!(e))
        .ok();

    tldr_core::compute_taint_with_tree(
        &cfg,
        &dfg.refs,
        &statements,
        tree.as_ref(),
        Some(file_contents.as_bytes()),
        language,
    )
    .map_err(|e| anyhow::anyhow!(e))
    .with_context(|| format!("Taint analysis for {}", function_id))
}

/// Run abstract interpretation for a specific function.
///
/// Builds the CFG and DFG, then runs the abstract interpretation pass
/// to compute value ranges, nullability, and potential issues.
pub fn build_abstract_interp(
    file_contents: &str,
    function_id: &FunctionId,
    language: Language,
) -> Result<tldr_core::AbstractInterpInfo> {
    let cfg = build_cfg_for_function(file_contents, function_id, language)?;
    let dfg = build_dfg_for_function(file_contents, function_id, language)?;

    let source_lines: Vec<&str> = file_contents.lines().collect();
    let lang_str = match language {
        Language::Python => "python",
        Language::Rust => "rust",
        Language::JavaScript | Language::TypeScript => "javascript",
        Language::Go => "go",
        Language::Java => "java",
        _ => "unknown",
    };

    tldr_core::compute_abstract_interp(&cfg, &dfg, Some(&source_lines), lang_str)
        .map_err(|e| anyhow::anyhow!(e))
        .with_context(|| format!("Abstract interpretation for {}", function_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simple Python function for testing IR construction.
    const PYTHON_SOURCE: &str = "def do_work(x):\n    y = x + 1\n    return y\n";

    /// Simple Rust function for testing IR construction.
    const RUST_SOURCE: &str = "fn do_work() {\n    let x = 1;\n    let y = x + 1;\n    y\n}\n";

    #[test]
    fn test_build_cfg_for_python_function() {
        let fid = FunctionId::new("src/example.py", "do_work", 1);
        let result = build_cfg_for_function(PYTHON_SOURCE, &fid, Language::Python);

        assert!(
            result.is_ok(),
            "CFG construction should succeed for valid Python, got: {:?}",
            result.err()
        );
        let cfg = result.unwrap();
        assert_eq!(cfg.function, "do_work");
        assert!(!cfg.blocks.is_empty(), "CFG should have at least one block");
    }

    #[test]
    fn test_build_cfg_for_rust_function() {
        let fid = FunctionId::new("src/example.rs", "do_work", 1);
        let result = build_cfg_for_function(RUST_SOURCE, &fid, Language::Rust);

        assert!(
            result.is_ok(),
            "CFG construction should succeed for valid Rust, got: {:?}",
            result.err()
        );
        let cfg = result.unwrap();
        assert_eq!(cfg.function, "do_work");
    }

    #[test]
    fn test_build_cfg_nonexistent_function_returns_empty() {
        let fid = FunctionId::new("src/example.py", "nonexistent", 1);
        let result = build_cfg_for_function(PYTHON_SOURCE, &fid, Language::Python);

        // tldr_core returns an empty CFG when the function is not found
        assert!(result.is_ok(), "should return Ok with empty CFG");
        let cfg = result.unwrap();
        assert!(cfg.blocks.is_empty(), "CFG should have no blocks for nonexistent function");
    }

    #[test]
    fn test_build_dfg_for_python_function() {
        let fid = FunctionId::new("src/example.py", "do_work", 1);
        let result = build_dfg_for_function(PYTHON_SOURCE, &fid, Language::Python);

        assert!(
            result.is_ok(),
            "DFG construction should succeed for valid Python, got: {:?}",
            result.err()
        );
        let dfg = result.unwrap();
        assert_eq!(dfg.function, "do_work");
        assert!(!dfg.refs.is_empty(), "DFG should have variable references");
    }

    #[test]
    fn test_build_dfg_nonexistent_function_returns_error() {
        let fid = FunctionId::new("src/example.py", "missing", 1);
        let result = build_dfg_for_function(PYTHON_SOURCE, &fid, Language::Python);

        assert!(
            result.is_err(),
            "DFG should fail for nonexistent function"
        );
    }

    #[test]
    fn test_build_ssa_for_python_function() {
        let fid = FunctionId::new("src/example.py", "do_work", 1);
        let result = build_ssa_for_function(PYTHON_SOURCE, &fid, Language::Python);

        assert!(
            result.is_ok(),
            "SSA construction should succeed for valid Python, got: {:?}",
            result.err()
        );
        let ssa = result.unwrap();
        assert_eq!(ssa.function, "do_work");
    }

    #[test]
    fn test_build_taint_for_python_function() {
        let fid = FunctionId::new("src/example.py", "do_work", 1);
        let result = build_taint_for_function(PYTHON_SOURCE, &fid, Language::Python);

        assert!(
            result.is_ok(),
            "Taint analysis should succeed for valid Python, got: {:?}",
            result.err()
        );
        let taint = result.unwrap();
        assert_eq!(taint.function_name, "do_work");
    }

    #[test]
    fn test_build_abstract_interp_for_python_function() {
        let fid = FunctionId::new("src/example.py", "do_work", 1);
        let result = build_abstract_interp(PYTHON_SOURCE, &fid, Language::Python);

        assert!(
            result.is_ok(),
            "Abstract interp should succeed for valid Python, got: {:?}",
            result.err()
        );
        let ai = result.unwrap();
        assert_eq!(ai.function_name, "do_work");
    }

    #[test]
    fn test_error_context_contains_function_id() {
        let fid = FunctionId::new("src/special.py", "Foo::bar_method", 10);
        // DFG fails for nonexistent function -- error should contain context
        let result = build_dfg_for_function(PYTHON_SOURCE, &fid, Language::Python);

        assert!(result.is_err());
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(
            err_msg.contains("Foo::bar_method"),
            "error should contain function name, got: {}",
            err_msg
        );
    }
}
