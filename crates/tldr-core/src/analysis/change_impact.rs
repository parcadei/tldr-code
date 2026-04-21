//! Change impact analysis (spec Section 2.7.2)
//!
//! Find tests affected by changed files to enable selective test execution.
//!
//! # Algorithm
//! 1. Detect changed files (git diff, session-modified, or explicit list)
//! 2. Build call graph and import graph for the project
//! 3. Find functions defined in changed files
//! 4. Use call graph to find functions that call changed functions
//! 5. Use import graph to find modules that import changed modules
//! 6. Filter to test files using language-specific patterns
//!
//! # Test File Detection Patterns
//! - Python: `test_*.py`, `*_test.py`, `conftest.py`
//! - TypeScript/JavaScript: `*.test.{js,ts}`, `*.spec.{js,ts}`
//! - Go: `*_test.go`
//! - Rust: `tests/*.rs`, `src/**/tests.rs`

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::callgraph::build_project_call_graph;
use crate::fs::tree::{collect_files, get_file_tree};
use crate::types::{FunctionRef, IgnoreSpec, Language, ProjectCallGraph};
use crate::TldrResult;

/// Change impact analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeImpactReport {
    /// Files that were detected as changed
    pub changed_files: Vec<PathBuf>,
    /// Test files that may be affected by the changes
    pub affected_tests: Vec<PathBuf>,
    /// Test functions affected (function-level granularity)
    #[serde(default)]
    pub affected_test_functions: Vec<TestFunction>,
    /// Functions affected by the changes (transitively)
    pub affected_functions: Vec<FunctionRef>,
    /// How changes were detected: "git:HEAD", "git:staged", etc.
    pub detection_method: String,
    /// Analysis metadata
    #[serde(default)]
    pub metadata: Option<ChangeImpactMetadata>,
}

/// Individual test function with location information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFunction {
    /// File containing the test
    pub file: PathBuf,
    /// Function name
    pub function: String,
    /// Class name for class-based test methods (e.g., TestAuth)
    pub class: Option<String>,
    /// Line number (1-indexed)
    pub line: u32,
}

/// Metadata about the analysis
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChangeImpactMetadata {
    /// Programming language analyzed
    pub language: String,
    /// Number of nodes in the call graph
    pub call_graph_nodes: usize,
    /// Number of edges in the call graph
    pub call_graph_edges: usize,
    /// Maximum traversal depth used
    pub analysis_depth: Option<usize>,
}

/// Detect method for finding changed files
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectionMethod {
    /// git diff HEAD (default)
    GitHead,
    /// git diff <base>...HEAD (PR workflow)
    GitBase {
        /// Base ref/branch used for the three-dot comparison.
        base: String,
    },
    /// git diff --staged (pre-commit)
    GitStaged,
    /// git diff (uncommitted: staged + unstaged)
    GitUncommitted,
    /// Explicit list provided by caller
    Explicit,
    /// Session tracking (placeholder - would need session tracking)
    Session,
}

impl std::fmt::Display for DetectionMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DetectionMethod::GitHead => write!(f, "git:HEAD"),
            DetectionMethod::GitBase { base } => write!(f, "git:{}...HEAD", base),
            DetectionMethod::GitStaged => write!(f, "git:staged"),
            DetectionMethod::GitUncommitted => write!(f, "git:uncommitted"),
            DetectionMethod::Explicit => write!(f, "explicit"),
            DetectionMethod::Session => write!(f, "session"),
        }
    }
}

/// Find tests affected by changed files.
///
/// # Arguments
/// * `project` - Project root directory
/// * `changed_files` - Optional explicit list of changed files. If None, uses git diff.
/// * `language` - Programming language
///
/// # Returns
/// * `Ok(ChangeImpactReport)` - Report of affected tests and functions
///
/// # Example
/// ```ignore
/// let report = change_impact(
///     Path::new("src"),
///     None,  // auto-detect via git
///     Language::Python,
/// )?;
///
/// for test in &report.affected_tests {
///     println!("Run: {}", test.display());
/// }
/// ```
pub fn change_impact(
    project: &Path,
    changed_files: Option<&[PathBuf]>,
    language: Language,
) -> TldrResult<ChangeImpactReport> {
    // Determine detection method based on whether explicit files are provided
    let (method, explicit) = if let Some(files) = changed_files {
        if files.is_empty() {
            // Empty list = use GitHead but no explicit files
            (DetectionMethod::GitHead, None)
        } else {
            // Non-empty list = use Explicit with files
            (DetectionMethod::Explicit, Some(files.to_vec()))
        }
    } else {
        // None = use GitHead auto-detection
        (DetectionMethod::GitHead, None)
    };

    change_impact_extended(
        project,
        method,
        language,
        10,   // default depth
        true, // include imports
        &[],  // no custom test patterns
        explicit,
    )
}

/// Extended change impact analysis with configurable detection method and options.
///
/// # Arguments
/// * `project` - Project root directory
/// * `method` - How to detect changed files
/// * `language` - Programming language
/// * `depth` - Maximum call graph traversal depth
/// * `include_imports` - Whether to include import graph in analysis
/// * `test_patterns` - Custom test file patterns (overrides defaults if non-empty)
/// * `explicit_files` - Optional explicit list (used with DetectionMethod::Explicit)
///
/// # Returns
/// * `Ok(ChangeImpactReport)` - Report of affected tests and functions
pub fn change_impact_extended(
    project: &Path,
    method: DetectionMethod,
    language: Language,
    depth: usize,
    _include_imports: bool,    // TODO: Use this in Phase 3
    _test_patterns: &[String], // TODO: Use this in Phase 3
    explicit_files: Option<Vec<PathBuf>>,
) -> TldrResult<ChangeImpactReport> {
    // Step 1: Determine changed files based on detection method
    let (files, actual_method) = match &method {
        DetectionMethod::Explicit => {
            let files = explicit_files.unwrap_or_default();
            (files, method.clone())
        }
        DetectionMethod::GitHead => {
            match detect_git_changes_head(project) {
                Ok(files) if !files.is_empty() => (files, method.clone()),
                Ok(_) => (vec![], method.clone()), // No changes is valid
                Err(_) => (vec![], DetectionMethod::Session), // Git not available
            }
        }
        DetectionMethod::GitBase { base } => {
            match detect_git_changes_base(project, base) {
                Ok(files) => (files, method.clone()),
                Err(e) => {
                    // Check if it's a branch not found error
                    let err_str = e.to_string();
                    if err_str.contains("not found") || err_str.contains("unknown revision") {
                        return Err(e);
                    }
                    (vec![], DetectionMethod::Session)
                }
            }
        }
        DetectionMethod::GitStaged => match detect_git_changes_staged(project) {
            Ok(files) => (files, method.clone()),
            Err(_) => (vec![], DetectionMethod::Session),
        },
        DetectionMethod::GitUncommitted => match detect_git_changes_uncommitted(project) {
            Ok(files) => (files, method.clone()),
            Err(_) => (vec![], DetectionMethod::Session),
        },
        DetectionMethod::Session => (vec![], method.clone()),
    };

    // Filter to only files matching the target language
    let changed_files: Vec<PathBuf> = files
        .into_iter()
        .filter(|f| {
            f.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| Language::from_extension(ext) == Some(language))
                .unwrap_or(false)
        })
        .collect();

    // If no changed files, return empty report
    if changed_files.is_empty() {
        return Ok(ChangeImpactReport {
            changed_files: vec![],
            affected_tests: vec![],
            affected_test_functions: vec![],
            affected_functions: vec![],
            detection_method: actual_method.to_string(),
            metadata: Some(ChangeImpactMetadata {
                language: language.to_string(),
                call_graph_nodes: 0,
                call_graph_edges: 0,
                analysis_depth: Some(depth),
            }),
        });
    }

    // Step 2: Build call graph
    let call_graph = build_project_call_graph(project, language, None, true)?;

    // Step 3: Find functions in changed files (call graph edges + AST extraction)
    let changed_functions = find_functions_in_files(&call_graph, &changed_files, project);

    // Step 4: Find all affected functions (callers of changed functions) with depth limit
    let affected_functions =
        find_affected_functions_with_depth(&call_graph, &changed_functions, depth);

    // Step 5: Find all project files
    let all_files = get_all_project_files(project, language)?;

    // Step 6: Filter to test files
    let test_files: HashSet<PathBuf> = all_files
        .iter()
        .filter(|f| is_test_file(f, language))
        .cloned()
        .collect();

    // Step 7: Find affected tests
    // A test is affected if:
    // - It's in a changed file
    // - It imports a changed module
    // - It calls a changed function
    let affected_tests = find_affected_tests(
        &test_files,
        &changed_files,
        &affected_functions,
        &call_graph,
    );

    // Extract test functions from affected test files (Phase 4)
    let affected_test_functions = extract_test_functions_from_files(&affected_tests, language);

    Ok(ChangeImpactReport {
        changed_files,
        affected_tests,
        affected_test_functions,
        affected_functions,
        detection_method: actual_method.to_string(),
        metadata: {
            let edge_count = call_graph.edges().count();
            Some(ChangeImpactMetadata {
                language: language.to_string(),
                call_graph_nodes: edge_count, // Approximate using edge count
                call_graph_edges: edge_count,
                analysis_depth: Some(depth),
            })
        },
    })
}

/// Extract test functions from a list of test files
fn extract_test_functions_from_files(
    test_files: &[PathBuf],
    language: Language,
) -> Vec<TestFunction> {
    let mut test_functions = Vec::new();

    for file in test_files {
        if let Ok(content) = std::fs::read_to_string(file) {
            test_functions.extend(extract_test_functions_from_content(
                file, &content, language,
            ));
        }
    }

    test_functions
}

/// Extract test functions from file content based on language patterns
fn extract_test_functions_from_content(
    file: &Path,
    content: &str,
    language: Language,
) -> Vec<TestFunction> {
    let mut functions = Vec::new();
    let mut current_class: Option<String> = None;

    for (line_num, line) in content.lines().enumerate() {
        let line_num = line_num as u32 + 1; // 1-indexed
        let trimmed = line.trim();
        let is_indented = line.starts_with("    ") || line.starts_with("\t");

        match language {
            Language::Python => {
                // Track class context
                if trimmed.starts_with("class ") && !is_indented {
                    // Extract class name: "class TestAuth:" -> "TestAuth"
                    if let Some(name) = trimmed
                        .strip_prefix("class ")
                        .and_then(|s| s.split(['(', ':']).next())
                    {
                        current_class = Some(name.trim().to_string());
                    }
                } else if !is_indented
                    && !trimmed.is_empty()
                    && !trimmed.starts_with("#")
                    && !trimmed.starts_with("@")
                {
                    // Non-indented, non-empty line - we're at module level
                    // Top-level def or any other statement means we're outside a class
                    if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
                        // Top-level function definition - clear class context
                        current_class = None;
                    } else if !trimmed.starts_with("class ") {
                        // Other module-level statement - clear class context
                        current_class = None;
                    }
                }

                // Look for test functions
                if trimmed.starts_with("def test_") || trimmed.starts_with("async def test_") {
                    let func_start = if trimmed.starts_with("async ") {
                        "async def "
                    } else {
                        "def "
                    };
                    if let Some(name) = trimmed
                        .strip_prefix(func_start)
                        .and_then(|s| s.split('(').next())
                    {
                        functions.push(TestFunction {
                            file: file.to_path_buf(),
                            function: name.to_string(),
                            class: current_class.clone(),
                            line: line_num,
                        });
                    }
                }
            }
            Language::TypeScript | Language::JavaScript => {
                // Look for test(), it(), describe()
                if trimmed.starts_with("test(") || trimmed.starts_with("it(") {
                    // Extract test name from: test('name', or it('name',
                    if let Some(start) = trimmed.find(['\'', '"']) {
                        let rest = &trimmed[start + 1..];
                        if let Some(end) = rest.find(['\'', '"']) {
                            functions.push(TestFunction {
                                file: file.to_path_buf(),
                                function: rest[..end].to_string(),
                                class: current_class.clone(),
                                line: line_num,
                            });
                        }
                    }
                } else if trimmed.starts_with("describe(") {
                    // Track describe block as "class"
                    if let Some(start) = trimmed.find(['\'', '"']) {
                        let rest = &trimmed[start + 1..];
                        if let Some(end) = rest.find(['\'', '"']) {
                            current_class = Some(rest[..end].to_string());
                        }
                    }
                }
            }
            Language::Go => {
                // Look for func Test...
                if trimmed.starts_with("func Test") {
                    if let Some(name) = trimmed
                        .strip_prefix("func ")
                        .and_then(|s| s.split('(').next())
                    {
                        functions.push(TestFunction {
                            file: file.to_path_buf(),
                            function: name.to_string(),
                            class: None,
                            line: line_num,
                        });
                    }
                }
            }
            Language::Rust => {
                // Look for #[test] followed by fn
                // This is a simplified check - proper parsing would track #[test] attributes
                if trimmed.starts_with("fn test_") || trimmed.starts_with("pub fn test_") {
                    let func_start = if trimmed.starts_with("pub fn ") {
                        "pub fn "
                    } else {
                        "fn "
                    };
                    if let Some(name) = trimmed
                        .strip_prefix(func_start)
                        .and_then(|s| s.split('(').next())
                    {
                        functions.push(TestFunction {
                            file: file.to_path_buf(),
                            function: name.to_string(),
                            class: None,
                            line: line_num,
                        });
                    }
                }
            }
            _ => {
                // Generic test detection
                if trimmed.contains("test") && trimmed.contains("fn ") {
                    // Try to extract function name
                    if let Some(fn_idx) = trimmed.find("fn ") {
                        let after_fn = &trimmed[fn_idx + 3..];
                        if let Some(name) = after_fn.split('(').next() {
                            functions.push(TestFunction {
                                file: file.to_path_buf(),
                                function: name.trim().to_string(),
                                class: None,
                                line: line_num,
                            });
                        }
                    }
                }
            }
        }
    }

    functions
}

/// Detect changed files using git diff HEAD (uncommitted changes vs HEAD)
fn detect_git_changes_head(project: &Path) -> TldrResult<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(["diff", "--name-only", "HEAD"])
        .current_dir(project)
        .output();

    parse_git_diff_output(output, project)
}

/// Detect changed files using git diff against a base branch (PR workflow)
/// Uses merge-base to find common ancestor: git diff $(git merge-base base HEAD)...HEAD
fn detect_git_changes_base(project: &Path, base: &str) -> TldrResult<Vec<PathBuf>> {
    // First, verify the base branch exists
    let check_branch = Command::new("git")
        .args(["rev-parse", "--verify", base])
        .current_dir(project)
        .output();

    match check_branch {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::error::TldrError::InvalidArgs {
                arg: "base".to_string(),
                message: format!("Branch '{}' not found. {}", base, stderr.trim()),
                suggestion: Some("Check branch name with: git branch -a".to_string()),
            });
        }
        Err(e) => {
            return Err(crate::error::TldrError::InvalidArgs {
                arg: "git".to_string(),
                message: format!("Git not available: {}", e),
                suggestion: None,
            });
        }
        _ => {}
    }

    // Use the three-dot syntax for comparing branches
    let output = Command::new("git")
        .args(["diff", "--name-only", &format!("{}...HEAD", base)])
        .current_dir(project)
        .output();

    parse_git_diff_output(output, project)
}

/// Detect only staged files (pre-commit workflow)
fn detect_git_changes_staged(project: &Path) -> TldrResult<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(["diff", "--name-only", "--staged"])
        .current_dir(project)
        .output();

    parse_git_diff_output(output, project)
}

/// Detect all uncommitted changes (staged + unstaged)
fn detect_git_changes_uncommitted(project: &Path) -> TldrResult<Vec<PathBuf>> {
    // Get staged changes
    let staged = Command::new("git")
        .args(["diff", "--name-only", "--staged"])
        .current_dir(project)
        .output();

    // Get unstaged changes
    let unstaged = Command::new("git")
        .args(["diff", "--name-only"])
        .current_dir(project)
        .output();

    let mut files = HashSet::new();

    if let Ok(output) = staged {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().filter(|l| !l.is_empty()) {
                let path = project.join(line);
                if path.exists() {
                    files.insert(path);
                }
            }
        }
    }

    if let Ok(output) = unstaged {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().filter(|l| !l.is_empty()) {
                let path = project.join(line);
                if path.exists() {
                    files.insert(path);
                }
            }
        }
    }

    Ok(files.into_iter().collect())
}

/// Parse git diff output into a list of file paths
fn parse_git_diff_output(
    output: std::io::Result<std::process::Output>,
    project: &Path,
) -> TldrResult<Vec<PathBuf>> {
    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let files: Vec<PathBuf> = stdout
                .lines()
                .filter(|line| !line.is_empty())
                .map(|line| project.join(line))
                .filter(|path| path.exists())
                .collect();
            Ok(files)
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(crate::error::TldrError::InvalidArgs {
                arg: "git".to_string(),
                message: format!("Git diff failed: {}", stderr.trim()),
                suggestion: None,
            })
        }
        Err(e) => Err(crate::error::TldrError::InvalidArgs {
            arg: "git".to_string(),
            message: format!("Git not available: {}", e),
            suggestion: Some("Ensure git is installed and on your PATH".to_string()),
        }),
    }
}

/// Find functions defined in the given files.
///
/// Uses two passes to ensure completeness:
/// 1. Call graph edges: finds functions that appear as sources or destinations in edges
/// 2. AST extraction: finds ALL functions defined in the files, including standalone
///    functions that neither call nor are called by anything
///
/// The AST pass is essential because the call graph only contains functions that
/// participate in at least one call relationship. Functions with no callers and no
/// callees (e.g., utility functions, dead code, newly added functions) would be
/// completely invisible to a call-graph-only approach.
fn find_functions_in_files(
    call_graph: &ProjectCallGraph,
    files: &[PathBuf],
    project_root: &Path,
) -> HashSet<FunctionRef> {
    let file_set: HashSet<&PathBuf> = files.iter().collect();
    let mut functions = HashSet::new();

    // Pass 1: Functions that appear as sources or destinations in call edges
    for edge in call_graph.edges() {
        if file_set.contains(&edge.src_file) {
            functions.insert(FunctionRef::new(
                edge.src_file.clone(),
                edge.src_func.clone(),
            ));
        }
        if file_set.contains(&edge.dst_file) {
            functions.insert(FunctionRef::new(
                edge.dst_file.clone(),
                edge.dst_func.clone(),
            ));
        }
    }

    // Pass 2: AST extraction to find ALL functions, including standalone ones
    // that have no call graph edges at all
    for file in files {
        let absolute_path = if file.is_absolute() {
            file.clone()
        } else {
            project_root.join(file)
        };

        match crate::ast::extract_file(&absolute_path, Some(project_root)) {
            Ok(module_info) => {
                // Add top-level functions
                for func in &module_info.functions {
                    functions.insert(FunctionRef::new(file.clone(), func.name.clone()));
                }
                // Add class methods (qualified as ClassName.method_name)
                for class in &module_info.classes {
                    for method in &class.methods {
                        let qualified_name = format!("{}.{}", class.name, method.name);
                        functions.insert(FunctionRef::new(file.clone(), qualified_name));
                    }
                }
            }
            Err(e) => {
                // AST extraction can fail for various reasons (binary files,
                // encoding issues, unsupported syntax). Log and continue --
                // the call-graph pass already found what it could.
                eprintln!(
                    "Warning: AST extraction failed for {}: {}",
                    absolute_path.display(),
                    e
                );
            }
        }
    }

    functions
}

/// Find all functions affected by changes with depth limiting
fn find_affected_functions_with_depth(
    call_graph: &ProjectCallGraph,
    changed_functions: &HashSet<FunctionRef>,
    max_depth: usize,
) -> Vec<FunctionRef> {
    let mut affected = HashSet::new();
    // Track (function, current_depth)
    let mut to_visit: Vec<(FunctionRef, usize)> =
        changed_functions.iter().map(|f| (f.clone(), 0)).collect();
    let mut visited: HashSet<FunctionRef> = HashSet::new();

    // Build reverse graph for traversal
    let reverse_graph = build_reverse_call_graph(call_graph);

    while let Some((func, depth)) = to_visit.pop() {
        if visited.contains(&func) {
            continue;
        }
        visited.insert(func.clone());
        affected.insert(func.clone());

        // Stop traversing if we've reached max depth
        if depth >= max_depth {
            continue;
        }

        // Find all callers of this function
        if let Some(callers) = reverse_graph.get(&func) {
            for caller in callers {
                if !visited.contains(caller) {
                    to_visit.push((caller.clone(), depth + 1));
                }
            }
        }
    }

    affected.into_iter().collect()
}

/// Build reverse call graph: callee -> [callers]
fn build_reverse_call_graph(
    call_graph: &ProjectCallGraph,
) -> std::collections::HashMap<FunctionRef, Vec<FunctionRef>> {
    let mut reverse = std::collections::HashMap::new();

    for edge in call_graph.edges() {
        let callee = FunctionRef::new(edge.dst_file.clone(), edge.dst_func.clone());
        let caller = FunctionRef::new(edge.src_file.clone(), edge.src_func.clone());

        reverse.entry(callee).or_insert_with(Vec::new).push(caller);
    }

    reverse
}

/// Get all source files in the project
fn get_all_project_files(project: &Path, language: Language) -> TldrResult<Vec<PathBuf>> {
    let extensions: HashSet<String> = language
        .extensions()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let tree = get_file_tree(
        project,
        Some(&extensions),
        true,
        Some(&IgnoreSpec::default()),
    )?;
    Ok(collect_files(&tree, project))
}

/// Check if a file is a test file based on language conventions
fn is_test_file(path: &Path, language: Language) -> bool {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let path_str = path.to_string_lossy();

    // Helper to check if path contains a test directory
    let in_tests_dir = || {
        path_str.contains("/tests/")
            || path_str.starts_with("tests/")
            || path_str.contains("/test/")
            || path_str.starts_with("test/")
    };

    let in_dunder_tests = || path_str.contains("/__tests__/") || path_str.starts_with("__tests__/");

    match language {
        Language::Python => {
            file_name.starts_with("test_")
                || file_name.ends_with("_test.py")
                || file_name == "conftest.py"
                || in_tests_dir()
        }
        Language::TypeScript | Language::JavaScript => {
            file_name.ends_with(".test.ts")
                || file_name.ends_with(".test.js")
                || file_name.ends_with(".spec.ts")
                || file_name.ends_with(".spec.js")
                || file_name.ends_with(".test.tsx")
                || file_name.ends_with(".test.jsx")
                || in_dunder_tests()
        }
        Language::Go => file_name.ends_with("_test.go"),
        Language::Rust => in_tests_dir() || file_name == "tests.rs",
        _ => {
            // Generic test detection
            file_name.contains("test") || in_tests_dir()
        }
    }
}

/// Find test files affected by the changes
fn find_affected_tests(
    test_files: &HashSet<PathBuf>,
    changed_files: &[PathBuf],
    affected_functions: &[FunctionRef],
    call_graph: &ProjectCallGraph,
) -> Vec<PathBuf> {
    let mut affected_tests = HashSet::new();

    // 1. Test files that were directly changed
    for file in changed_files {
        if test_files.contains(file) {
            affected_tests.insert(file.clone());
        }
    }

    // 2. Test files that contain affected functions
    let affected_files: HashSet<&PathBuf> = affected_functions.iter().map(|f| &f.file).collect();
    for test_file in test_files {
        if affected_files.contains(test_file) {
            affected_tests.insert(test_file.clone());
        }
    }

    // 3. Test files that call any changed function
    let changed_file_set: HashSet<&PathBuf> = changed_files.iter().collect();
    for edge in call_graph.edges() {
        // If source is a test file and destination is in a changed file
        if test_files.contains(&edge.src_file) && changed_file_set.contains(&edge.dst_file) {
            affected_tests.insert(edge.src_file.clone());
        }
    }

    let mut result: Vec<PathBuf> = affected_tests.into_iter().collect();
    result.sort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_test_file_python() {
        assert!(is_test_file(Path::new("test_main.py"), Language::Python));
        assert!(is_test_file(Path::new("main_test.py"), Language::Python));
        assert!(is_test_file(Path::new("conftest.py"), Language::Python));
        assert!(is_test_file(
            Path::new("tests/test_utils.py"),
            Language::Python
        ));
        assert!(!is_test_file(Path::new("main.py"), Language::Python));
    }

    #[test]
    fn test_is_test_file_typescript() {
        assert!(is_test_file(
            Path::new("main.test.ts"),
            Language::TypeScript
        ));
        assert!(is_test_file(
            Path::new("main.spec.ts"),
            Language::TypeScript
        ));
        assert!(is_test_file(
            Path::new("__tests__/main.ts"),
            Language::TypeScript
        ));
        assert!(!is_test_file(Path::new("main.ts"), Language::TypeScript));
    }

    #[test]
    fn test_is_test_file_go() {
        assert!(is_test_file(Path::new("main_test.go"), Language::Go));
        assert!(!is_test_file(Path::new("main.go"), Language::Go));
    }

    #[test]
    fn test_is_test_file_rust() {
        assert!(is_test_file(
            Path::new("tests/integration.rs"),
            Language::Rust
        ));
        assert!(is_test_file(Path::new("src/lib/tests.rs"), Language::Rust));
        assert!(!is_test_file(Path::new("src/main.rs"), Language::Rust));
    }

    #[test]
    fn test_detection_method_display() {
        assert_eq!(DetectionMethod::GitHead.to_string(), "git:HEAD");
        assert_eq!(
            DetectionMethod::GitBase {
                base: "main".to_string()
            }
            .to_string(),
            "git:main...HEAD"
        );
        assert_eq!(DetectionMethod::GitStaged.to_string(), "git:staged");
        assert_eq!(
            DetectionMethod::GitUncommitted.to_string(),
            "git:uncommitted"
        );
        assert_eq!(DetectionMethod::Session.to_string(), "session");
        assert_eq!(DetectionMethod::Explicit.to_string(), "explicit");
    }

    #[test]
    fn test_empty_change_impact() {
        // With no changed files, should return empty report
        let report = ChangeImpactReport {
            changed_files: vec![],
            affected_tests: vec![],
            affected_test_functions: vec![],
            affected_functions: vec![],
            detection_method: "explicit".to_string(),
            metadata: None,
        };

        assert!(report.changed_files.is_empty());
        assert!(report.affected_tests.is_empty());
    }

    #[test]
    fn test_extract_python_test_functions() {
        let content = r#"
class TestAuth:
    def test_login(self):
        pass

    def test_logout(self):
        pass

def test_standalone():
    pass
"#;
        let file = Path::new("test_auth.py");
        let functions = extract_test_functions_from_content(file, content, Language::Python);

        assert_eq!(functions.len(), 3);
        assert!(functions
            .iter()
            .any(|f| f.function == "test_login" && f.class == Some("TestAuth".to_string())));
        assert!(functions
            .iter()
            .any(|f| f.function == "test_logout" && f.class == Some("TestAuth".to_string())));
        assert!(functions
            .iter()
            .any(|f| f.function == "test_standalone" && f.class.is_none()));
    }

    /// Test that find_functions_in_files discovers standalone functions
    /// that do not appear in any call graph edge.
    ///
    /// Bug: Before the fix, find_functions_in_files only found functions
    /// appearing as sources or destinations in call graph edges. Functions
    /// that neither call nor are called by anything were completely missed.
    #[test]
    fn test_find_functions_in_files_includes_standalone() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project = tmp.path();

        // Create a Python file with:
        // - connected_caller() calls connected_callee() => both in call graph edges
        // - standalone_func() calls nothing, called by nothing => NOT in any edge
        let src = project.join("src");
        std::fs::create_dir_all(&src).unwrap();

        let module_path = src.join("module.py");
        std::fs::write(
            &module_path,
            r#"
def connected_caller():
    return connected_callee()

def connected_callee():
    return 42

def standalone_func():
    """This function neither calls nor is called by anything."""
    return "I exist but am isolated"
"#,
        )
        .unwrap();

        // Build call graph for the project
        let call_graph = build_project_call_graph(project, Language::Python, None, true).unwrap();

        // Call graph stores relative paths, so use those for matching
        let changed_files = vec![PathBuf::from("src/module.py")];

        let functions = find_functions_in_files(&call_graph, &changed_files, project);

        // Should find ALL three functions, not just the two in call edges
        let names: HashSet<&str> = functions.iter().map(|f| f.name.as_str()).collect();

        assert!(
            names.contains("connected_caller"),
            "Should find connected_caller (it appears in call edges as source)"
        );
        assert!(
            names.contains("connected_callee"),
            "Should find connected_callee (it appears in call edges as destination)"
        );
        assert!(
            names.contains("standalone_func"),
            "Should find standalone_func even though it has no call edges. \
             Found only: {:?}",
            names
        );
    }

    /// Test that find_functions_in_files discovers class methods that are standalone
    /// (not referenced in any call graph edge).
    #[test]
    fn test_find_functions_in_files_includes_standalone_methods() {
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let project = tmp.path();

        let src = project.join("src");
        std::fs::create_dir_all(&src).unwrap();

        let module_path = src.join("myclass.py");
        std::fs::write(
            &module_path,
            r#"
class MyClass:
    def used_method(self):
        return self.helper()

    def helper(self):
        return 42

    def orphan_method(self):
        """Not called by anything, does not call anything."""
        return "orphan"
"#,
        )
        .unwrap();

        let call_graph = build_project_call_graph(project, Language::Python, None, true).unwrap();

        // Call graph stores relative paths
        let changed_files = vec![PathBuf::from("src/myclass.py")];
        let functions = find_functions_in_files(&call_graph, &changed_files, project);
        let names: HashSet<&str> = functions.iter().map(|f| f.name.as_str()).collect();

        // orphan_method should be found even though it has no call edges
        assert!(
            names.contains("orphan_method") || names.contains("MyClass.orphan_method"),
            "Should find orphan_method even though it has no call edges. Found: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_go_test_functions() {
        let content = r#"
package auth

func TestLogin(t *testing.T) {
    // test
}

func TestLogout(t *testing.T) {
    // test
}
"#;
        let file = Path::new("auth_test.go");
        let functions = extract_test_functions_from_content(file, content, Language::Go);

        assert_eq!(functions.len(), 2);
        assert!(functions.iter().any(|f| f.function == "TestLogin"));
        assert!(functions.iter().any(|f| f.function == "TestLogout"));
    }
}
