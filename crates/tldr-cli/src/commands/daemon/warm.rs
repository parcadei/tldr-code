//! Warm command implementation
//!
//! CLI command: `tldr warm PATH [--background] [--lang LANG]`
//!
//! Pre-builds call graph cache for faster subsequent queries.
//!
//! # Behavior
//!
//! 1. If `--background`: spawn detached process, return immediately
//! 2. Foreground mode: build call graph synchronously
//! 3. If daemon is running: send Warm command via IPC
//! 4. If daemon not running and background: start daemon then warm
//!
//! # Output
//!
//! JSON format:
//! ```json
//! {
//!   "status": "ok",
//!   "files": 150,
//!   "edges": 2500,
//!   "languages": ["python", "typescript"],
//!   "cache_path": ".tldr/cache/call_graph.json"
//! }
//! ```

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use clap::Args;
use regex::Regex;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::output::OutputFormat;

use super::error::{DaemonError, DaemonResult};
use super::ipc::{check_socket_alive, send_command};
use super::types::DaemonCommand;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the `warm` command.
#[derive(Debug, Clone, Args)]
pub struct WarmArgs {
    /// Project root directory to warm
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Run warming in background process
    #[arg(long, short = 'b')]
    pub background: bool,
    // Note: Use global --lang to specify language, or auto-detect if not specified
}

// =============================================================================
// Output Types
// =============================================================================

/// Output structure for successful warm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarmOutput {
    /// Status message
    pub status: String,
    /// Number of files indexed
    pub files: usize,
    /// Number of call graph edges
    pub edges: usize,
    /// Languages detected/analyzed
    pub languages: Vec<String>,
    /// Path to the cache file
    pub cache_path: PathBuf,
}

/// Call graph edge for serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    pub from_file: PathBuf,
    pub from_func: String,
    pub to_file: PathBuf,
    pub to_func: String,
}

/// Call graph cache file format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraphCache {
    pub edges: Vec<CallEdge>,
    pub languages: Vec<String>,
    pub timestamp: i64,
}

// =============================================================================
// Implementation
// =============================================================================

impl WarmArgs {
    /// Run the warm command.
    pub fn run(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Create a new tokio runtime for the async operations
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(self.run_async(format, quiet))
    }

    /// Async implementation of the warm command.
    async fn run_async(&self, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
        // Resolve project path to absolute
        let project = self.path.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(&self.path)
        });

        if self.background {
            // Run in background
            self.run_background(&project, format, quiet).await
        } else {
            // Check if daemon is running - if so, send command via IPC
            if check_socket_alive(&project).await {
                self.run_via_daemon(&project, format, quiet).await
            } else {
                // Run synchronously in foreground
                self.run_foreground(&project, format, quiet).await
            }
        }
    }

    /// Run warming in background (spawn detached process).
    async fn run_background(
        &self,
        project: &Path,
        format: OutputFormat,
        quiet: bool,
    ) -> anyhow::Result<()> {
        // Spawn detached process
        let exe = std::env::current_exe()?;
        let mut cmd = StdCommand::new(exe);
        cmd.arg("warm").arg(project.to_str().unwrap_or("."));

        // Language auto-detection happens in the background process

        // On Unix, we use setsid to detach
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        // On Windows, use CREATE_NO_WINDOW and DETACHED_PROCESS
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            const DETACHED_PROCESS: u32 = 0x00000008;
            cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
        }

        cmd.spawn()?;

        // Output background message
        if !quiet {
            match format {
                OutputFormat::Json | OutputFormat::Compact => {
                    let output = serde_json::json!({
                        "status": "ok",
                        "message": "Warming cache in background..."
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                    println!("Warming cache in background...");
                }
            }
        }

        Ok(())
    }

    /// Run warming via IPC to running daemon.
    async fn run_via_daemon(
        &self,
        project: &Path,
        format: OutputFormat,
        quiet: bool,
    ) -> anyhow::Result<()> {
        let cmd = DaemonCommand::Warm {
            language: None, // Auto-detect
        };

        let response = send_command(project, &cmd)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send warm command to daemon: {}", e))?;

        if !quiet {
            match format {
                OutputFormat::Json | OutputFormat::Compact => {
                    println!("{}", serde_json::to_string_pretty(&response)?);
                }
                OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                    println!("Warm command sent to daemon");
                }
            }
        }

        Ok(())
    }

    /// Run warming synchronously in foreground.
    async fn run_foreground(
        &self,
        project: &Path,
        format: OutputFormat,
        quiet: bool,
    ) -> anyhow::Result<()> {
        if !quiet {
            match format {
                OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                    println!("Warming call graph cache...");
                }
                _ => {}
            }
        }

        // Ensure .tldr directory exists
        let tldr_dir = project.join(".tldr");
        fs::create_dir_all(&tldr_dir)?;

        // Ensure .tldrignore exists
        let ignore_path = project.join(".tldrignore");
        if !ignore_path.exists() {
            fs::write(
                &ignore_path,
                "# TLDR ignore file\n\
                 .git/\n\
                 node_modules/\n\
                 __pycache__/\n\
                 target/\n\
                 build/\n\
                 dist/\n\
                 .venv/\n\
                 venv/\n\
                 *.pyc\n\
                 *.pyo\n",
            )?;
        }

        // Auto-detect languages
        let languages = detect_languages(project)?;

        // Build call graph
        let (files, edges) = build_call_graph(project, &languages)?;

        // Write cache file
        let cache_dir = tldr_dir.join("cache");
        fs::create_dir_all(&cache_dir)?;
        let cache_path = cache_dir.join("call_graph.json");

        let cache = CallGraphCache {
            edges: edges.clone(),
            languages: languages.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        };

        fs::write(&cache_path, serde_json::to_string_pretty(&cache)?)?;

        // Output result
        let output = WarmOutput {
            status: "ok".to_string(),
            files,
            edges: edges.len(),
            languages,
            cache_path: PathBuf::from(".tldr/cache/call_graph.json"),
        };

        // Always output result (quiet only suppresses progress messages)
        match format {
            OutputFormat::Json | OutputFormat::Compact => {
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            OutputFormat::Text | OutputFormat::Sarif | OutputFormat::Dot => {
                println!(
                    "Indexed {} files, found {} edges",
                    output.files, output.edges
                );
                println!("Languages: {}", output.languages.join(", "));
                println!("Cache written to: {}", output.cache_path.display());
            }
        }

        Ok(())
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Hardcoded directory names to always skip during warm walks.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "__pycache__",
    "target",
    "build",
    "dist",
    "venv",
    ".venv",
];

/// Load ignore patterns from `.tldrignore` file in the project root.
/// Returns directory stems to skip (e.g., "corpus" from "corpus/").
fn load_tldrignore(project: &Path) -> HashSet<String> {
    let mut patterns = HashSet::new();
    let ignore_path = project.join(".tldrignore");
    if let Ok(content) = fs::read_to_string(&ignore_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // Strip trailing slash for directory patterns
            let name = trimmed.trim_end_matches('/');
            if !name.is_empty() {
                patterns.insert(name.to_string());
            }
        }
    }
    patterns
}

/// Check if a path component should be skipped (hidden, hardcoded, or .tldrignore).
fn should_skip_component(component: &str, ignore_patterns: &HashSet<String>) -> bool {
    component.starts_with('.')
        || SKIP_DIRS.contains(&component)
        || ignore_patterns.contains(component)
}

/// Detect languages present in the project.
fn detect_languages(project: &Path) -> anyhow::Result<Vec<String>> {
    let mut languages = HashSet::new();
    let ignore_patterns = load_tldrignore(project);

    // Walk directory looking for language-specific files
    for entry in WalkDir::new(project)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_dir() && e.depth() > 0 {
                let name = e.file_name().to_string_lossy();
                !should_skip_component(&name, &ignore_patterns)
            } else {
                true
            }
        })
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                match ext_str.as_str() {
                    "py" => {
                        languages.insert("python".to_string());
                    }
                    "ts" | "tsx" => {
                        languages.insert("typescript".to_string());
                    }
                    "js" | "jsx" => {
                        languages.insert("javascript".to_string());
                    }
                    "rs" => {
                        languages.insert("rust".to_string());
                    }
                    "go" => {
                        languages.insert("go".to_string());
                    }
                    "java" => {
                        languages.insert("java".to_string());
                    }
                    "rb" => {
                        languages.insert("ruby".to_string());
                    }
                    "cpp" | "cc" | "cxx" | "hpp" | "h" => {
                        languages.insert("cpp".to_string());
                    }
                    "c" => {
                        languages.insert("c".to_string());
                    }
                    _ => {}
                }
            }
        }
    }

    let mut result: Vec<_> = languages.into_iter().collect();
    result.sort();

    if result.is_empty() {
        result.push("unknown".to_string());
    }

    Ok(result)
}

/// Build call graph for the project.
///
/// Returns (file_count, edges).
fn build_call_graph(
    project: &Path,
    languages: &[String],
) -> anyhow::Result<(usize, Vec<CallEdge>)> {
    let mut file_count = 0;
    let mut edges = Vec::new();

    // Get language extensions to filter
    let extensions: HashSet<&str> = languages
        .iter()
        .flat_map(|lang| match lang.as_str() {
            "python" => vec!["py"],
            "typescript" => vec!["ts", "tsx"],
            "javascript" => vec!["js", "jsx"],
            "rust" => vec!["rs"],
            "go" => vec!["go"],
            "java" => vec!["java"],
            "ruby" => vec!["rb"],
            "cpp" => vec!["cpp", "cc", "cxx", "hpp", "h"],
            "c" => vec!["c", "h"],
            _ => vec![],
        })
        .collect();

    // Walk project and extract function definitions and calls, respecting .tldrignore
    let ignore_patterns = load_tldrignore(project);
    for entry in WalkDir::new(project)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_dir() && e.depth() > 0 {
                let name = e.file_name().to_string_lossy();
                !should_skip_component(&name, &ignore_patterns)
            } else {
                true
            }
        })
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if extensions.contains(ext_str.as_str()) {
                    file_count += 1;

                    // Extract call edges from this file
                    if let Ok(content) = fs::read_to_string(path) {
                        let file_edges = extract_call_edges(path, &content, &ext_str);
                        edges.extend(file_edges);
                    }
                }
            }
        }
    }

    Ok((file_count, edges))
}

/// Extract call edges from a source file.
///
/// This is a simplified regex-based implementation.
/// Production code would use tree-sitter for accurate parsing.
fn extract_call_edges(file_path: &std::path::Path, content: &str, lang: &str) -> Vec<CallEdge> {
    let mut edges = Vec::new();
    let mut current_func: Option<String> = None;

    // Simple function/method detection patterns
    let func_pattern = match lang {
        "py" => Regex::new(r"^\s*def\s+(\w+)\s*\(").ok(),
        "ts" | "tsx" | "js" | "jsx" => {
            Regex::new(r"(?:function\s+(\w+)|(\w+)\s*(?::\s*\w+)?\s*=\s*(?:async\s+)?(?:function|\([^)]*\)\s*=>))").ok()
        }
        "rs" => Regex::new(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)").ok(),
        "go" => Regex::new(r"^\s*func\s+(?:\([^)]+\)\s+)?(\w+)\s*\(").ok(),
        "java" => Regex::new(r"^\s*(?:public|private|protected)?\s*(?:static)?\s*\w+\s+(\w+)\s*\(").ok(),
        "rb" => Regex::new(r"^\s*def\s+(\w+)").ok(),
        _ => None,
    };

    // Simple call detection pattern (function calls)
    let call_pattern = Regex::new(r"\b(\w+)\s*\(").ok();

    for line in content.lines() {
        // Check for function definition
        if let Some(ref pattern) = func_pattern {
            if let Some(caps) = pattern.captures(line) {
                // Get the first non-None capture group
                current_func = caps
                    .iter()
                    .skip(1)
                    .flatten()
                    .next()
                    .map(|m| m.as_str().to_string());
            }
        }

        // Check for function calls within current function
        if let (Some(ref current), Some(ref pattern)) = (&current_func, &call_pattern) {
            for caps in pattern.captures_iter(line) {
                if let Some(call) = caps.get(1) {
                    let call_name = call.as_str();
                    // Skip common keywords and builtins
                    if !is_builtin_or_keyword(call_name) && call_name != current {
                        edges.push(CallEdge {
                            from_file: file_path.to_path_buf(),
                            from_func: current.clone(),
                            to_file: file_path.to_path_buf(), // Simplified: assume same file
                            to_func: call_name.to_string(),
                        });
                    }
                }
            }
        }
    }

    edges
}

/// Check if a name is a builtin or language keyword.
fn is_builtin_or_keyword(name: &str) -> bool {
    let common_builtins = [
        "if",
        "else",
        "for",
        "while",
        "return",
        "print",
        "len",
        "str",
        "int",
        "float",
        "bool",
        "list",
        "dict",
        "set",
        "tuple",
        "range",
        "enumerate",
        "zip",
        "map",
        "filter",
        "sorted",
        "reversed",
        "sum",
        "min",
        "max",
        "abs",
        "round",
        "type",
        "isinstance",
        "issubclass",
        "hasattr",
        "getattr",
        "setattr",
        "delattr",
        "open",
        "close",
        "read",
        "write",
        "append",
        "extend",
        "insert",
        "remove",
        "pop",
        "clear",
        "copy",
        "update",
        "get",
        "keys",
        "values",
        "items",
        "join",
        "split",
        "strip",
        "replace",
        "format",
        "console",
        "log",
        "require",
        "import",
        "export",
        "const",
        "let",
        "var",
        "new",
        "this",
        "self",
        "super",
        "class",
        "struct",
        "impl",
        "trait",
        "pub",
        "fn",
        "async",
        "await",
        "match",
        "Some",
        "None",
        "Ok",
        "Err",
        "Vec",
        "String",
        "Box",
        "Arc",
        "Rc",
        "Mutex",
        "Result",
        "Option",
    ];

    common_builtins.contains(&name)
}

/// Public function to run warm command (for daemon integration).
pub async fn cmd_warm(args: WarmArgs) -> DaemonResult<WarmOutput> {
    // Resolve project path
    let project = args.path.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&args.path)
    });

    // Auto-detect languages
    let languages = detect_languages(&project).map_err(|e| {
        DaemonError::Io(std::io::Error::other(e.to_string()))
    })?;

    // Build call graph
    let (files, edges) = build_call_graph(&project, &languages).map_err(|e| {
        DaemonError::Io(std::io::Error::other(e.to_string()))
    })?;

    // Write cache file
    let cache_dir = project.join(".tldr/cache");
    fs::create_dir_all(&cache_dir).map_err(DaemonError::Io)?;
    let cache_path = cache_dir.join("call_graph.json");

    let cache = CallGraphCache {
        edges: edges.clone(),
        languages: languages.clone(),
        timestamp: chrono::Utc::now().timestamp(),
    };

    fs::write(&cache_path, serde_json::to_string_pretty(&cache)?)?;

    Ok(WarmOutput {
        status: "ok".to_string(),
        files,
        edges: edges.len(),
        languages,
        cache_path: PathBuf::from(".tldr/cache/call_graph.json"),
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_warm_args_default() {
        let args = WarmArgs {
            path: PathBuf::from("."),
            background: false,
        };

        assert_eq!(args.path, PathBuf::from("."));
        assert!(!args.background);
    }

    #[test]
    fn test_warm_args_with_options() {
        let args = WarmArgs {
            path: PathBuf::from("/test/project"),
            background: true,
        };

        assert!(args.background);
    }

    #[test]
    fn test_warm_output_serialization() {
        let output = WarmOutput {
            status: "ok".to_string(),
            files: 150,
            edges: 2500,
            languages: vec!["python".to_string(), "typescript".to_string()],
            cache_path: PathBuf::from(".tldr/cache/call_graph.json"),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("ok"));
        assert!(json.contains("150"));
        assert!(json.contains("2500"));
        assert!(json.contains("python"));
    }

    #[test]
    fn test_detect_languages() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("main.py"), "def main(): pass").unwrap();
        fs::write(temp.path().join("app.ts"), "function main() {}").unwrap();

        let languages = detect_languages(temp.path()).unwrap();
        assert!(languages.contains(&"python".to_string()));
        assert!(languages.contains(&"typescript".to_string()));
    }

    #[test]
    fn test_detect_languages_empty() {
        let temp = TempDir::new().unwrap();
        let languages = detect_languages(temp.path()).unwrap();
        assert_eq!(languages, vec!["unknown".to_string()]);
    }

    #[test]
    fn test_build_call_graph_python() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("main.py"),
            "def main():\n    helper()\n\ndef helper():\n    pass",
        )
        .unwrap();

        let (files, edges) =
            build_call_graph(temp.path(), &["python".to_string()]).unwrap();

        assert_eq!(files, 1);
        assert!(!edges.is_empty());
        // Should have edge from main -> helper
        assert!(edges
            .iter()
            .any(|e| e.from_func == "main" && e.to_func == "helper"));
    }

    #[test]
    fn test_extract_call_edges_python() {
        let content = "def foo():\n    bar()\n    baz(1, 2)\n";
        let edges = extract_call_edges(std::path::Path::new("test.py"), content, "py");

        assert!(edges
            .iter()
            .any(|e| e.from_func == "foo" && e.to_func == "bar"));
        assert!(edges
            .iter()
            .any(|e| e.from_func == "foo" && e.to_func == "baz"));
    }

    #[test]
    fn test_is_builtin_or_keyword() {
        assert!(is_builtin_or_keyword("print"));
        assert!(is_builtin_or_keyword("len"));
        assert!(is_builtin_or_keyword("if"));
        assert!(!is_builtin_or_keyword("my_function"));
    }

    #[test]
    fn test_call_graph_cache_serialization() {
        let cache = CallGraphCache {
            edges: vec![CallEdge {
                from_file: PathBuf::from("main.py"),
                from_func: "main".to_string(),
                to_file: PathBuf::from("utils.py"),
                to_func: "helper".to_string(),
            }],
            languages: vec!["python".to_string()],
            timestamp: 1234567890,
        };

        let json = serde_json::to_string(&cache).unwrap();
        assert!(json.contains("main.py"));
        assert!(json.contains("helper"));
        assert!(json.contains("1234567890"));
    }

    // =========================================================================
    // Property-based tests (proptest)
    // =========================================================================

    mod proptest_warm {
        use super::*;
        use proptest::prelude::*;

        /// Generate a valid directory component name (no /, no NUL).
        fn arb_component() -> impl Strategy<Value = String> {
            prop::string::string_regex("[a-zA-Z0-9_.][a-zA-Z0-9_.-]{0,15}")
                .unwrap()
        }

        proptest! {
            /// Invariant: should_skip_component never panics on arbitrary input.
            #[test]
            fn skip_component_no_panic(component in ".*") {
                let patterns = HashSet::new();
                let _ = should_skip_component(&component, &patterns);
            }

            /// Invariant: hidden dirs (starting with .) are always skipped.
            #[test]
            fn hidden_dirs_always_skipped(name in "\\.[a-zA-Z0-9_]{1,20}") {
                let patterns = HashSet::new();
                prop_assert!(should_skip_component(&name, &patterns),
                    "'{}' starts with '.' but was not skipped", name);
            }

            /// Invariant: patterns in ignore set are always skipped.
            #[test]
            fn ignore_patterns_always_skipped(
                name in arb_component(),
                extra in prop::collection::hash_set(arb_component(), 0..5),
            ) {
                let mut patterns = extra;
                patterns.insert(name.clone());
                prop_assert!(should_skip_component(&name, &patterns),
                    "'{}' is in ignore set but was not skipped", name);
            }

            /// Invariant: detect_languages never panics on a temp dir with
            /// arbitrary file names.
            #[test]
            fn detect_languages_no_panic(
                files in prop::collection::vec(
                    (arb_component(), prop::sample::select(vec!["py", "ts", "rs", "go", "rb", "txt", ""])),
                    0..10
                )
            ) {
                let temp = TempDir::new().unwrap();
                for (name, ext) in &files {
                    let filename = if ext.is_empty() {
                        name.clone()
                    } else {
                        format!("{}.{}", name, ext)
                    };
                    let _ = fs::write(temp.path().join(&filename), "content");
                }
                let result = detect_languages(temp.path());
                prop_assert!(result.is_ok(), "detect_languages should not fail");
                let langs = result.unwrap();
                prop_assert!(!langs.is_empty(), "should return at least 'unknown'");
            }
        }
    }
}
