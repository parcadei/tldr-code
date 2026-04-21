//! Context builder (spec Section 2.7.1)
//!
//! Builds LLM-ready context from an entry point via BFS traversal of the call graph.
//!
//! # Algorithm
//! 1. Build cross-file call graph for the project
//! 2. Find the entry point function in the graph
//! 3. BFS traverse callees to specified depth
//! 4. For each function: extract signature, optionally docstring, CFG metrics
//! 5. Format as LLM-consumable text
//!
//! # Token Savings
//! Instead of reading entire files, we extract only:
//! - Function signatures
//! - Docstrings (optional)
//! - Call relationships
//! - CFG metrics (blocks, cyclomatic complexity)
//!
//! This achieves ~95% token savings compared to reading full files.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ast::extract::extract_file;
use crate::callgraph::build_project_call_graph;
use crate::cfg::get_cfg_context;
use crate::error::TldrError;
use crate::types::{FunctionInfo, Language, ProjectCallGraph};
use crate::TldrResult;

/// Context information for a single function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionContext {
    /// Function name
    pub name: String,
    /// File containing the function (relative to project root)
    pub file: PathBuf,
    /// Line number where function is defined
    pub line: u32,
    /// Function signature (e.g., "def foo(x: int, y: str) -> bool")
    pub signature: String,
    /// Optional docstring
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// Functions called by this function
    pub calls: Vec<String>,
    /// Number of basic blocks in CFG
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<usize>,
    /// Cyclomatic complexity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyclomatic: Option<u32>,
}

/// Relevant context for LLM consumption
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevantContext {
    /// Entry point function name
    pub entry_point: String,
    /// Traversal depth used
    pub depth: usize,
    /// All functions in context (entry point + callees)
    pub functions: Vec<FunctionContext>,
}

impl RelevantContext {
    /// Format context for LLM consumption
    ///
    /// Produces a human-readable format suitable for including in LLM prompts.
    pub fn to_llm_string(&self) -> String {
        let mut output = String::new();

        output.push_str(&format!(
            "# Code Context: {} (depth={})\n\n",
            self.entry_point, self.depth
        ));

        output.push_str(&format!(
            "## Summary\n- Entry point: `{}`\n- Functions included: {}\n\n",
            self.entry_point,
            self.functions.len()
        ));

        output.push_str("## Functions\n\n");

        for func in &self.functions {
            output.push_str(&format!(
                "### {} ({}:{})\n\n",
                func.name,
                func.file.display(),
                func.line
            ));
            output.push_str(&format!("```\n{}\n```\n\n", func.signature));

            if let Some(ref doc) = func.docstring {
                output.push_str(&format!("**Docstring:** {}\n\n", doc.trim()));
            }

            if !func.calls.is_empty() {
                output.push_str(&format!("**Calls:** {}\n\n", func.calls.join(", ")));
            }

            if let (Some(blocks), Some(cyclomatic)) = (func.blocks, func.cyclomatic) {
                output.push_str(&format!(
                    "**Complexity:** {} blocks, cyclomatic={}\n\n",
                    blocks, cyclomatic
                ));
            }

            output.push_str("---\n\n");
        }

        output
    }
}

/// Get relevant context for LLM starting from an entry point.
///
/// # Arguments
/// * `project` - Project root directory
/// * `entry_point` - Name of the entry point function
/// * `depth` - Maximum traversal depth (0 = entry only, 1 = entry + direct callees, etc.)
/// * `language` - Programming language
/// * `include_docstrings` - Whether to include docstrings in output
/// * `file_filter` - Optional file path to disambiguate common function names.
///   When provided, only matches functions in files whose path ends with this filter.
///   For example, `Some(Path::new("django/shortcuts.py"))` selects `render` from that
///   specific file when multiple files define a function named `render`.
///
/// # Returns
/// * `Ok(RelevantContext)` - Context with functions and their metadata
/// * `Err(TldrError::FunctionNotFound)` - Entry point not found
///
/// # Example
/// ```ignore
/// let ctx = get_relevant_context(
///     Path::new("src"),
///     "main",
///     2,
///     Language::Python,
///     true,
///     None, // no file filter
/// )?;
/// ```
pub fn get_relevant_context(
    project: &Path,
    entry_point: &str,
    depth: usize,
    language: Language,
    include_docstrings: bool,
    file_filter: Option<&Path>,
) -> TldrResult<RelevantContext> {
    // Step 1: Build call graph
    let call_graph = build_project_call_graph(project, language, None, true)?;

    // Step 2: Find entry point in the graph
    let entry_location = find_function_in_graph(&call_graph, entry_point, project, file_filter)?;

    // Step 3: BFS traversal to collect all functions up to depth
    let function_keys = bfs_collect_functions(&call_graph, &entry_location, depth);

    // Step 4: Extract context for each function
    let mut functions = Vec::new();
    let mut seen_files: HashMap<PathBuf, crate::types::ModuleInfo> = HashMap::new();

    for (file, func_name) in function_keys {
        // The call graph stores relative paths (e.g., "main.py", "lib/utils.py").
        // Resolve them against the project root so extract_file() can canonicalize.
        let full_path = if file.is_relative() {
            project.join(&file)
        } else {
            file.clone()
        };

        // Cache file extractions (keyed on original relative path for consistency)
        let module_info = if let Some(info) = seen_files.get(&file) {
            info.clone()
        } else {
            let info = extract_file(&full_path, Some(project)).unwrap_or_else(|_| {
                // Return empty module info on error
                crate::types::ModuleInfo {
                    file_path: file.clone(),
                    language,
                    docstring: None,
                    imports: vec![],
                    functions: vec![],
                    classes: vec![],
                    constants: vec![],
                    call_graph: Default::default(),
                }
            });
            seen_files.insert(file.clone(), info.clone());
            info
        };

        // Find the function in the module
        if let Some(func_info) = find_function_info(&module_info, &func_name) {
            let func_context = build_function_context(
                &full_path,
                &func_name,
                func_info,
                &module_info,
                project,
                language,
                include_docstrings,
            );
            functions.push(func_context);
        }
    }

    Ok(RelevantContext {
        entry_point: entry_point.to_string(),
        depth,
        functions,
    })
}

/// Find a function's location (file, name) in the call graph.
///
/// When `file_filter` is `Some`, only matches functions whose file path ends with
/// the filter path. This disambiguates common function names that appear in multiple files.
fn find_function_in_graph(
    call_graph: &ProjectCallGraph,
    func_name: &str,
    project: &Path,
    file_filter: Option<&Path>,
) -> TldrResult<(PathBuf, String)> {
    // Helper: check if a file path matches the filter
    let file_matches = |file: &Path| -> bool {
        match file_filter {
            None => true,
            Some(filter) => file.ends_with(filter),
        }
    };

    // First, check if function appears as source or destination in any edge
    for edge in call_graph.edges() {
        if (edge.src_func == func_name || edge.src_func.ends_with(&format!(".{}", func_name)))
            && file_matches(&edge.src_file)
        {
            return Ok((edge.src_file.clone(), edge.src_func.clone()));
        }
        if (edge.dst_func == func_name || edge.dst_func.ends_with(&format!(".{}", func_name)))
            && file_matches(&edge.dst_file)
        {
            return Ok((edge.dst_file.clone(), edge.dst_func.clone()));
        }
    }

    // If not in call graph, it might be a standalone function
    // Try to find it by scanning project files
    if let Some(location) = scan_project_for_function(project, func_name, file_filter)? {
        return Ok(location);
    }

    // Not found - collect suggestions
    let suggestions = collect_similar_function_names(call_graph, func_name);

    Err(TldrError::FunctionNotFound {
        name: func_name.to_string(),
        file: None,
        suggestions,
    })
}

/// Scan project files to find a function by name.
///
/// When `file_filter` is `Some`, only scans files whose path ends with the filter,
/// narrowing the search to a specific file for disambiguation.
fn scan_project_for_function(
    project: &Path,
    func_name: &str,
    file_filter: Option<&Path>,
) -> TldrResult<Option<(PathBuf, String)>> {
    use crate::fs::tree::{collect_files, get_file_tree};
    use crate::types::IgnoreSpec;

    // Get all source files
    let tree = get_file_tree(project, None, true, Some(&IgnoreSpec::default()))?;
    let files = collect_files(&tree, project);

    for file_path in files {
        // If file_filter is set, skip files that don't match
        if let Some(filter) = file_filter {
            // Check if the file path (relative to project) ends with the filter
            let relative = file_path.strip_prefix(project).unwrap_or(&file_path);
            if !relative.ends_with(filter) {
                continue;
            }
        }

        if let Ok(module_info) = extract_file(&file_path, Some(project)) {
            // Check top-level functions
            for func in &module_info.functions {
                if func.name == func_name {
                    return Ok(Some((file_path, func.name.clone())));
                }
            }
            // Check class methods
            for class in &module_info.classes {
                for method in &class.methods {
                    if method.name == func_name {
                        let full_name = format!("{}.{}", class.name, method.name);
                        return Ok(Some((file_path, full_name)));
                    }
                }
            }
        }
    }

    Ok(None)
}

/// Collect similar function names for error suggestions
fn collect_similar_function_names(call_graph: &ProjectCallGraph, target: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut suggestions = Vec::new();
    let target_lower = target.to_lowercase();

    for edge in call_graph.edges() {
        for func in [&edge.src_func, &edge.dst_func] {
            if !seen.contains(func) {
                seen.insert(func.clone());
                let func_lower = func.to_lowercase();
                // Simple similarity: contains or edit distance would be better
                if func_lower.contains(&target_lower) || target_lower.contains(&func_lower) {
                    suggestions.push(func.clone());
                }
            }
        }
    }

    suggestions.sort();
    suggestions.truncate(5);
    suggestions
}

/// BFS collect all functions from entry point up to depth
fn bfs_collect_functions(
    call_graph: &ProjectCallGraph,
    entry: &(PathBuf, String),
    max_depth: usize,
) -> Vec<(PathBuf, String)> {
    let mut result = Vec::new();
    let mut visited: HashSet<(PathBuf, String)> = HashSet::new();
    let mut queue: VecDeque<((PathBuf, String), usize)> = VecDeque::new();

    // Build forward graph: caller -> [callees]
    let forward_graph = build_forward_graph(call_graph);

    // Start with entry point
    queue.push_back((entry.clone(), 0));
    visited.insert(entry.clone());

    while let Some(((file, func), current_depth)) = queue.pop_front() {
        result.push((file.clone(), func.clone()));

        // Don't explore further if at max depth
        if current_depth >= max_depth {
            continue;
        }

        // Find all callees of this function
        let key = (file.clone(), func.clone());
        if let Some(callees) = forward_graph.get(&key) {
            for callee in callees {
                if !visited.contains(callee) {
                    visited.insert(callee.clone());
                    queue.push_back((callee.clone(), current_depth + 1));
                }
            }
        }
    }

    result
}

/// Build forward graph: (src_file, src_func) -> [(dst_file, dst_func)]
fn build_forward_graph(
    call_graph: &ProjectCallGraph,
) -> HashMap<(PathBuf, String), Vec<(PathBuf, String)>> {
    let mut forward: HashMap<(PathBuf, String), Vec<(PathBuf, String)>> = HashMap::new();

    for edge in call_graph.edges() {
        let src_key = (edge.src_file.clone(), edge.src_func.clone());
        let dst_key = (edge.dst_file.clone(), edge.dst_func.clone());

        forward.entry(src_key).or_default().push(dst_key);
    }

    forward
}

/// Find function info in a module
fn find_function_info<'a>(
    module_info: &'a crate::types::ModuleInfo,
    func_name: &str,
) -> Option<&'a FunctionInfo> {
    // Check top-level functions
    for func in &module_info.functions {
        if func.name == func_name {
            return Some(func);
        }
    }

    // Check class methods (func_name might be "ClassName.method")
    if let Some(dot_idx) = func_name.find('.') {
        let class_name = &func_name[..dot_idx];
        let method_name = &func_name[dot_idx + 1..];

        for class in &module_info.classes {
            if class.name == class_name {
                for method in &class.methods {
                    if method.name == method_name {
                        return Some(method);
                    }
                }
            }
        }
    }

    None
}

/// Build function context with all metadata
fn build_function_context(
    file: &Path,
    func_name: &str,
    func_info: &FunctionInfo,
    module_info: &crate::types::ModuleInfo,
    project: &Path,
    language: Language,
    include_docstrings: bool,
) -> FunctionContext {
    // Build signature
    let signature = build_signature(func_info, language);

    // Get calls from intra-file call graph
    let calls = module_info
        .call_graph
        .calls
        .get(&func_info.name)
        .cloned()
        .unwrap_or_default();

    // Get CFG metrics (best effort - don't fail if unavailable)
    let (blocks, cyclomatic) = get_cfg_metrics(file, func_name, language);

    // Make file path relative to project
    let relative_file = file
        .strip_prefix(project)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| file.to_path_buf());

    FunctionContext {
        name: func_name.to_string(),
        file: relative_file,
        line: func_info.line_number,
        signature,
        docstring: if include_docstrings {
            func_info.docstring.clone()
        } else {
            None
        },
        calls,
        blocks,
        cyclomatic,
    }
}

/// Build function signature string
fn build_signature(func_info: &FunctionInfo, language: Language) -> String {
    let params = func_info.params.join(", ");

    let return_type = func_info
        .return_type
        .as_ref()
        .map(|t| format!(" -> {}", t))
        .unwrap_or_default();

    let async_prefix = if func_info.is_async { "async " } else { "" };

    match language {
        Language::Python => {
            format!(
                "{}def {}({}){}",
                async_prefix, func_info.name, params, return_type
            )
        }
        Language::TypeScript | Language::JavaScript => {
            format!(
                "{}function {}({}){}",
                async_prefix, func_info.name, params, return_type
            )
        }
        Language::Go => {
            format!("func {}({}){}", func_info.name, params, return_type)
        }
        Language::Rust => {
            format!(
                "{}fn {}({}){}",
                async_prefix, func_info.name, params, return_type
            )
        }
        _ => {
            format!("{}({}){}", func_info.name, params, return_type)
        }
    }
}

/// Get CFG metrics for a function
fn get_cfg_metrics(
    file: &Path,
    func_name: &str,
    language: Language,
) -> (Option<usize>, Option<u32>) {
    // Extract just the function name without class prefix for CFG lookup
    let lookup_name = if let Some(dot_idx) = func_name.rfind('.') {
        &func_name[dot_idx + 1..]
    } else {
        func_name
    };

    match get_cfg_context(file.to_str().unwrap_or(""), lookup_name, language) {
        Ok(cfg) => (Some(cfg.blocks.len()), Some(cfg.cyclomatic_complexity)),
        Err(_) => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that get_relevant_context resolves relative paths from the call graph
    /// correctly against the project root, producing non-empty function context.
    ///
    /// This is a regression test for a bug where BFS returned relative paths
    /// (e.g., "main.py") but extract_file() canonicalized them relative to CWD
    /// instead of the project root, causing silent extraction failures.
    #[test]
    fn test_context_resolves_relative_paths_from_callgraph() {
        use std::fs;
        use tempfile::TempDir;

        // Create a temp project with two files where main.py calls helper.py
        let temp_dir = TempDir::new().unwrap();
        let project = temp_dir.path();

        // main.py: imports and calls helper from helper.py
        let main_py = r#"from helper import do_work

def main():
    """Entry point."""
    result = do_work(42)
    return result
"#;

        // helper.py: defines do_work which calls internal_calc
        let helper_py = r#"def do_work(x):
    """Do some work."""
    return internal_calc(x) + 1

def internal_calc(x):
    """Internal calculation."""
    return x * 2
"#;

        fs::write(project.join("main.py"), main_py).unwrap();
        fs::write(project.join("helper.py"), helper_py).unwrap();

        // Get context starting from "main" with depth=1
        // This should find "main" and its callees (including cross-file "do_work")
        let result = get_relevant_context(project, "main", 1, Language::Python, true, None);

        assert!(
            result.is_ok(),
            "get_relevant_context failed: {:?}",
            result.err()
        );
        let ctx = result.unwrap();

        // The context should include functions (not be empty due to path resolution failure)
        assert!(
            !ctx.functions.is_empty(),
            "Expected non-empty functions in context, got 0. \
             This indicates extract_file() failed to resolve relative paths from the call graph."
        );

        // The entry point "main" should be present
        let func_names: Vec<&str> = ctx.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(
            func_names.contains(&"main"),
            "Expected 'main' in context functions, got: {:?}",
            func_names
        );

        // With depth=1, callees of main should also appear (e.g., do_work)
        assert!(
            func_names.contains(&"do_work"),
            "Expected callee 'do_work' in context at depth=1, got: {:?}",
            func_names
        );
    }

    /// Test that context works for intra-file calls (same file, different functions).
    /// This is the simpler case that may work even without the path fix if CWD happens
    /// to match the project root, so we test the cross-file case separately above.
    #[test]
    fn test_context_intra_file_calls() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let project = temp_dir.path();

        let main_py = r#"def entry():
    """Entry function."""
    return helper(10)

def helper(n):
    """Helper function."""
    return n + 1
"#;

        fs::write(project.join("main.py"), main_py).unwrap();

        let result = get_relevant_context(project, "entry", 1, Language::Python, true, None);

        assert!(
            result.is_ok(),
            "get_relevant_context failed: {:?}",
            result.err()
        );
        let ctx = result.unwrap();

        assert!(
            !ctx.functions.is_empty(),
            "Expected non-empty functions in context"
        );

        let func_names: Vec<&str> = ctx.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(
            func_names.contains(&"entry"),
            "Expected 'entry' in context, got: {:?}",
            func_names
        );
    }

    #[test]
    fn test_relevant_context_to_llm_string() {
        let ctx = RelevantContext {
            entry_point: "main".to_string(),
            depth: 1,
            functions: vec![
                FunctionContext {
                    name: "main".to_string(),
                    file: PathBuf::from("src/main.py"),
                    line: 10,
                    signature: "def main()".to_string(),
                    docstring: Some("Entry point".to_string()),
                    calls: vec!["helper".to_string()],
                    blocks: Some(3),
                    cyclomatic: Some(2),
                },
                FunctionContext {
                    name: "helper".to_string(),
                    file: PathBuf::from("src/utils.py"),
                    line: 5,
                    signature: "def helper(x: int) -> str".to_string(),
                    docstring: None,
                    calls: vec![],
                    blocks: Some(1),
                    cyclomatic: Some(1),
                },
            ],
        };

        let output = ctx.to_llm_string();
        assert!(output.contains("main"));
        assert!(output.contains("helper"));
        assert!(output.contains("Entry point"));
        assert!(output.contains("depth=1"));
    }

    #[test]
    fn test_build_signature_python() {
        let func = FunctionInfo {
            name: "process".to_string(),
            params: vec!["x: int".to_string(), "y: str".to_string()],
            return_type: Some("bool".to_string()),
            docstring: None,
            is_method: false,
            is_async: false,
            decorators: vec![],
            line_number: 1,
        };

        let sig = build_signature(&func, Language::Python);
        assert_eq!(sig, "def process(x: int, y: str) -> bool");
    }

    #[test]
    fn test_build_signature_async() {
        let func = FunctionInfo {
            name: "fetch".to_string(),
            params: vec!["url: str".to_string()],
            return_type: Some("Response".to_string()),
            docstring: None,
            is_method: false,
            is_async: true,
            decorators: vec![],
            line_number: 1,
        };

        let sig = build_signature(&func, Language::Python);
        assert_eq!(sig, "async def fetch(url: str) -> Response");
    }

    #[test]
    fn test_bfs_collect_empty_graph() {
        let graph = ProjectCallGraph::new();
        let entry = (PathBuf::from("main.py"), "main".to_string());
        let result = bfs_collect_functions(&graph, &entry, 5);
        // Entry point should always be included even if graph is empty
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "main");
    }

    /// Test that the `file_filter` parameter disambiguates functions with the same name
    /// across different files. When two files both define `render`, passing a file filter
    /// should select only the function from the specified file.
    #[test]
    fn test_file_filter_disambiguates_same_function_name() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let project = temp_dir.path();

        // Create two files that each define a function called "render"
        let shortcuts_py = r#"def render(request, template_name):
    """Shortcut render function."""
    return load_template(template_name)

def load_template(name):
    """Load a template by name."""
    return name
"#;

        let backends_py = r#"def render(template, context):
    """Backend render function."""
    return compile_template(template)

def compile_template(template):
    """Compile a template."""
    return template
"#;

        // Place in subdirectories to simulate django-like structure
        fs::create_dir_all(project.join("django")).unwrap();
        fs::write(project.join("django/shortcuts.py"), shortcuts_py).unwrap();
        fs::create_dir_all(project.join("django/template/backends")).unwrap();
        fs::write(
            project.join("django/template/backends/django.py"),
            backends_py,
        )
        .unwrap();

        // Without file_filter: should find *some* render (current behavior)
        let result_any = get_relevant_context(
            project,
            "render",
            1,
            Language::Python,
            false,
            None, // no file filter
        );
        assert!(
            result_any.is_ok(),
            "get_relevant_context without filter failed: {:?}",
            result_any.err()
        );
        let ctx_any = result_any.unwrap();
        assert!(
            !ctx_any.functions.is_empty(),
            "Expected non-empty functions without filter"
        );

        // With file_filter: select the render from django/shortcuts.py specifically
        let result_shortcuts = get_relevant_context(
            project,
            "render",
            1,
            Language::Python,
            false,
            Some(Path::new("django/shortcuts.py")),
        );
        assert!(
            result_shortcuts.is_ok(),
            "get_relevant_context with shortcuts filter failed: {:?}",
            result_shortcuts.err()
        );
        let ctx_shortcuts = result_shortcuts.unwrap();
        assert!(
            !ctx_shortcuts.functions.is_empty(),
            "Expected non-empty functions with shortcuts filter"
        );

        // The entry point should be from django/shortcuts.py
        let entry_func = &ctx_shortcuts.functions[0];
        assert_eq!(entry_func.name, "render");
        assert!(
            entry_func.file.ends_with("django/shortcuts.py"),
            "Expected render from django/shortcuts.py, got: {}",
            entry_func.file.display()
        );

        // With depth=1, should also include load_template (callee of shortcuts.render)
        let callee_names: Vec<&str> = ctx_shortcuts
            .functions
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert!(
            callee_names.contains(&"load_template"),
            "Expected callee 'load_template' from shortcuts, got: {:?}",
            callee_names
        );
        // Should NOT contain compile_template (that's in the backends file)
        assert!(
            !callee_names.contains(&"compile_template"),
            "Should not contain 'compile_template' from backends when filtering to shortcuts"
        );

        // With file_filter: select the render from django/template/backends/django.py
        let result_backends = get_relevant_context(
            project,
            "render",
            1,
            Language::Python,
            false,
            Some(Path::new("django/template/backends/django.py")),
        );
        assert!(
            result_backends.is_ok(),
            "get_relevant_context with backends filter failed: {:?}",
            result_backends.err()
        );
        let ctx_backends = result_backends.unwrap();
        let backend_entry = &ctx_backends.functions[0];
        assert_eq!(backend_entry.name, "render");
        assert!(
            backend_entry
                .file
                .ends_with("django/template/backends/django.py"),
            "Expected render from backends/django.py, got: {}",
            backend_entry.file.display()
        );

        // Should include compile_template but NOT load_template
        let backend_names: Vec<&str> = ctx_backends
            .functions
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert!(
            backend_names.contains(&"compile_template"),
            "Expected callee 'compile_template' from backends, got: {:?}",
            backend_names
        );
        assert!(
            !backend_names.contains(&"load_template"),
            "Should not contain 'load_template' from shortcuts when filtering to backends"
        );
    }

    /// Test that file_filter works with a nonexistent file path and returns FunctionNotFound
    #[test]
    fn test_file_filter_nonexistent_file_returns_error() {
        use std::fs;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let project = temp_dir.path();

        let main_py = r#"def render():
    """A render function."""
    pass
"#;
        fs::write(project.join("main.py"), main_py).unwrap();

        // Filter to a file that doesn't contain render
        let result = get_relevant_context(
            project,
            "render",
            0,
            Language::Python,
            false,
            Some(Path::new("nonexistent.py")),
        );

        assert!(
            result.is_err(),
            "Expected FunctionNotFound error when filtering to nonexistent file"
        );
    }
}
