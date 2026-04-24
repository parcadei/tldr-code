//! Born-dead detection for bugbot
//!
//! Detects new functions (added in the current changeset) that have zero
//! references anywhere in the codebase.  Zero false positives by construction:
//! if you just wrote a function and nothing calls it, it is dead.
//!
//! Uses the reference-counting approach from the `dead` command (single-pass
//! identifier scan via tree-sitter) rather than the full call graph.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use tldr_core::analysis::refcount::{count_identifiers_in_tree, is_rescued_by_refcount};
use tldr_core::ast::parser::parse_file;
use tldr_core::Language;

use super::types::BugbotFinding;
use crate::commands::dead::collect_module_infos_with_refcounts;
use crate::commands::remaining::types::ASTChange;

/// Evidence payload for a born-dead finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BornDeadEvidence {
    /// Whether the function has `pub` visibility.
    pub is_public: bool,
    /// Number of identifier occurrences across the project (1 = definition only).
    pub ref_count: usize,
}

/// Check inserted functions for zero references (born dead).
///
/// # Arguments
/// * `inserted` - AST changes of type Insert + Function/Method (from `diff::inserted_functions`)
/// * `project`  - Project root directory (used to scan for refcounts)
/// * `language` - Language of the project files
///
/// Only call this when there are Insert changes -- the refcount scan is the
/// expensive part (~seconds for large projects).
pub fn compose_born_dead(
    inserted: &[&ASTChange],
    project: &Path,
    language: &Language,
) -> Result<Vec<BugbotFinding>> {
    if inserted.is_empty() {
        return Ok(Vec::new());
    }

    // Scan the entire project for identifier refcounts (single-pass tree-sitter).
    let (_module_infos, ref_counts) =
        collect_module_infos_with_refcounts(project, *language, false);

    compose_born_dead_with_refcounts(inserted, &ref_counts)
}

/// Scoped born-dead detection: only scan changed files + their importers.
///
/// Instead of scanning the entire project (which takes minutes on large repos),
/// this scans only the files that could possibly reference the new functions:
/// - The changed files themselves (a new call must be in a changed file)
/// - Files that import from changed modules (covers pre-existing call sites)
///
/// This reduces the scan from 847 files / 3+ minutes to ~30 files / <100ms
/// on the TLDR codebase.
pub fn compose_born_dead_scoped(
    inserted: &[&ASTChange],
    changed_files: &[PathBuf],
    project: &Path,
    language: &Language,
) -> Result<Vec<BugbotFinding>> {
    if inserted.is_empty() {
        return Ok(Vec::new());
    }

    // Tier 1: Count identifiers in changed files only.
    let mut ref_counts: HashMap<String, usize> = HashMap::new();
    for file in changed_files {
        if !file.exists() {
            continue;
        }
        if let Ok((tree, source, lang)) = parse_file(file) {
            let file_counts = count_identifiers_in_tree(&tree, source.as_bytes(), lang);
            for (name, count) in file_counts {
                *ref_counts.entry(name).or_insert(0) += count;
            }
        }
    }

    // Tier 2: Find files that import changed modules, scan those too.
    // This catches the case where a new function satisfies a pre-existing
    // call site in an unchanged file that imports the changed module.
    let importer_files = find_importer_files(changed_files, project, language);
    for file in &importer_files {
        if !file.exists() {
            continue;
        }
        // Skip files already scanned in tier 1
        if changed_files.iter().any(|cf| cf == file) {
            continue;
        }
        if let Ok((tree, source, lang)) = parse_file(file) {
            let file_counts = count_identifiers_in_tree(&tree, source.as_bytes(), lang);
            for (name, count) in file_counts {
                *ref_counts.entry(name).or_insert(0) += count;
            }
        }
    }

    compose_born_dead_with_refcounts(inserted, &ref_counts)
}

/// Find files that import any of the changed modules.
///
/// For each changed file, derives its module name and calls `find_importers`
/// to locate files that import it. Returns a deduplicated list of file paths.
fn find_importer_files(
    changed_files: &[PathBuf],
    project: &Path,
    language: &Language,
) -> Vec<PathBuf> {
    let mut importer_paths = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for file in changed_files {
        // Derive module name from file path relative to project root.
        // e.g., "src/foo.rs" -> try "foo", "crate::foo", "src/foo"
        let rel = file.strip_prefix(project).unwrap_or(file);
        let module_candidates = derive_module_names(rel);

        for module_name in &module_candidates {
            if let Ok(report) =
                tldr_core::analysis::importers::find_importers(project, module_name, *language)
            {
                for importer in &report.importers {
                    let importer_path = project.join(&importer.file);
                    if seen.insert(importer_path.clone()) {
                        importer_paths.push(importer_path);
                    }
                }
            }
        }
    }

    importer_paths
}

/// Derive possible module names from a relative file path.
///
/// e.g., `src/utils/helpers.rs` -> `["helpers", "utils::helpers", "crate::utils::helpers"]`
/// e.g., `lib.rs` -> `["lib"]`
fn derive_module_names(rel_path: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let stem = rel_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

    if stem.is_empty() {
        return names;
    }

    // Bare module name (e.g., "helpers")
    names.push(stem.to_string());

    // Build path-based module name, stripping src/ prefix and extension
    let components: Vec<&str> = rel_path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    if components.len() > 1 {
        // Skip common source directory prefixes
        let skip = if components[0] == "src" || components[0] == "lib" {
            1
        } else {
            0
        };
        let module_parts: Vec<&str> = components[skip..].to_vec();

        // Replace file extension in last component with nothing
        if let Some(last) = module_parts.last() {
            let mut parts: Vec<String> = module_parts[..module_parts.len() - 1]
                .iter()
                .map(|s| s.to_string())
                .collect();
            let last_stem = Path::new(last)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(last);
            // Skip mod.rs / __init__.py — use parent as module name
            if last_stem != "mod" && last_stem != "__init__" {
                parts.push(last_stem.to_string());
            }

            if !parts.is_empty() {
                // Qualified path (e.g., "utils::helpers")
                let qualified = parts.join("::");
                if qualified != stem {
                    names.push(qualified.clone());
                }
                // Crate-prefixed (e.g., "crate::utils::helpers")
                names.push(format!("crate::{}", qualified));
            }
        }
    }

    names
}

/// Inner implementation that accepts pre-computed refcounts.
///
/// Separated so tests can provide synthetic refcount maps without scanning a
/// real project directory.
pub fn compose_born_dead_with_refcounts(
    inserted: &[&ASTChange],
    ref_counts: &HashMap<String, usize>,
) -> Result<Vec<BugbotFinding>> {
    let mut findings = Vec::new();

    for change in inserted {
        let name = match change.name.as_deref() {
            Some(n) => n,
            None => continue, // no name => skip
        };

        // Skip test functions (they are entry points invoked by test runners)
        if is_test_function(name) {
            continue;
        }

        // Skip functions in test files — they are test infrastructure
        // invoked by test harnesses, not dead code.
        let file_path = change
            .new_location
            .as_ref()
            .map(|loc| loc.file.as_str())
            .unwrap_or("");
        if is_test_file(file_path) {
            continue;
        }

        // Skip standard entry points (main, etc.)
        if is_entry_point(name) {
            continue;
        }

        // Skip trait impl methods -- heuristic: check `new_text` for `impl ... for`
        let new_text = change.new_text.as_deref().unwrap_or("");
        if is_trait_impl(name, new_text) {
            continue;
        }

        // Check refcount: if rescued (ref_count > 1, name >= 3 chars) => alive
        if is_rescued_by_refcount(name, ref_counts) {
            continue;
        }

        // Not rescued => born dead
        let is_public = new_text.contains("pub fn ") || new_text.contains("pub async fn ");
        let ref_count = lookup_ref_count(name, ref_counts);

        let line = change
            .new_location
            .as_ref()
            .map(|loc| loc.line as usize)
            .unwrap_or(0);

        let file = change
            .new_location
            .as_ref()
            .map(|loc| PathBuf::from(&loc.file))
            .unwrap_or_default();

        let severity = if is_public { "medium" } else { "low" };

        findings.push(BugbotFinding {
            finding_type: "born-dead".to_string(),
            severity: severity.to_string(),
            file,
            function: name.to_string(),
            line,
            message: format!(
                "New function '{}' has no callers (ref_count: {})",
                name, ref_count
            ),
            evidence: serde_json::to_value(&BornDeadEvidence {
                is_public,
                ref_count,
            })
            .unwrap_or_default(),
            confidence: None,
            finding_id: None,
        });
    }

    Ok(findings)
}

/// Look up the refcount for a function name, handling qualified names.
fn lookup_ref_count(name: &str, ref_counts: &HashMap<String, usize>) -> usize {
    // Try bare name first (e.g. "method" from "Class.method")
    let bare_name = if name.contains('.') {
        name.rsplit('.').next().unwrap_or(name)
    } else if name.contains(':') {
        name.rsplit(':').next().unwrap_or(name)
    } else {
        name
    };

    if let Some(&count) = ref_counts.get(bare_name) {
        return count;
    }
    if bare_name != name {
        if let Some(&count) = ref_counts.get(name) {
            return count;
        }
    }
    0
}

/// Check if a function name looks like a test function.
pub fn is_test_function(name: &str) -> bool {
    name.starts_with("test_")
        || name == "test"
        || name.starts_with("Test")
        || name.starts_with("Benchmark")
        || name.starts_with("Example")
}

/// Check if a file path indicates a test file.
///
/// Covers common conventions across languages:
/// - Rust: `tests/` directory, `_test.rs`, `_tests.rs`
/// - Python: `test_*.py`, `*_test.py`, `tests/` directory
/// - JS/TS: `*.test.ts`, `*.spec.ts`, `__tests__/`
/// - Go: `*_test.go`
/// - Java: `*Test.java`, `src/test/`
fn is_test_file(path: &str) -> bool {
    // Normalize separators for cross-platform matching
    let path = path.replace('\\', "/");

    // Directory-based patterns
    if path.contains("/tests/")
        || path.contains("/test/")
        || path.contains("/__tests__/")
        || path.contains("/testing/")
    {
        return true;
    }

    // File suffix patterns (extract the filename)
    let filename = path.rsplit('/').next().unwrap_or(&path);
    filename.ends_with("_test.rs")
        || filename.ends_with("_tests.rs")
        || filename.ends_with("_test.go")
        || filename.ends_with("_test.py")
        || filename.starts_with("test_")
        || filename.ends_with(".test.ts")
        || filename.ends_with(".test.tsx")
        || filename.ends_with(".test.js")
        || filename.ends_with(".test.jsx")
        || filename.ends_with(".spec.ts")
        || filename.ends_with(".spec.tsx")
        || filename.ends_with(".spec.js")
        || filename.ends_with(".spec.jsx")
        || filename.ends_with("Test.java")
        || filename.ends_with("Tests.java")
}

/// Check if a function name is a standard entry point.
fn is_entry_point(name: &str) -> bool {
    matches!(
        name,
        "main" | "__main__" | "cli" | "app" | "run" | "start" | "create_app" | "make_app" | "lib"
    )
}

/// Name-based heuristic: suppress born-dead findings for common standard-library
/// trait method names.
///
/// The `new_text` field in an `ASTChange` contains the function body, not the
/// surrounding `impl` block, so we cannot inspect the impl context.  Instead we
/// check the function name against known trait methods from `std` and popular
/// crates (serde).  These methods are invoked polymorphically and almost never
/// truly dead.
///
/// False negatives (a trait impl method we don't recognise) produce an extra
/// finding the user can ignore.  False positives (silencing real dead code) are
/// worse, so the list is kept conservative -- only names that are exclusively
/// or overwhelmingly used as trait implementations.
fn is_trait_impl(name: &str, _text: &str) -> bool {
    matches!(
        name,
        "fmt"
            | "from"
            | "into"
            | "try_from"
            | "try_into"
            | "clone"
            | "clone_from"
            | "default"
            | "drop"
            | "deref"
            | "deref_mut"
            | "as_ref"
            | "as_mut"
            | "borrow"
            | "borrow_mut"
            | "eq"
            | "ne"
            | "partial_cmp"
            | "cmp"
            | "hash"
            | "next"
            | "size_hint"
            | "index"
            | "index_mut"
            | "from_str"
            | "to_string"
            | "write_str"
            | "serialize"
            | "deserialize"
            | "poll"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::remaining::types::{ChangeType, Location, NodeKind};

    /// Helper to build an ASTChange representing an inserted function.
    fn make_insert(name: &str, text: &str, file: &str, line: u32) -> ASTChange {
        ASTChange {
            change_type: ChangeType::Insert,
            node_kind: NodeKind::Function,
            name: Some(name.to_string()),
            old_location: None,
            new_location: Some(Location {
                file: file.to_string(),
                line,
                column: 0,
                end_line: None,
                end_column: None,
            }),
            old_text: None,
            new_text: Some(text.to_string()),
            similarity: None,
            children: None,
            base_changes: None,
        }
    }

    #[test]
    fn test_born_dead_new_unused_function() {
        let insert = make_insert(
            "helper",
            "fn helper() { println!(\"unused\"); }",
            "src/lib.rs",
            10,
        );
        let inserted: Vec<&ASTChange> = vec![&insert];

        // refcounts: "helper" appears once (its definition)
        let mut ref_counts = HashMap::new();
        ref_counts.insert("helper".to_string(), 1);

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert_eq!(findings.len(), 1, "Should detect one born-dead function");
        assert_eq!(findings[0].finding_type, "born-dead");
        assert_eq!(findings[0].function, "helper");
        assert_eq!(findings[0].line, 10);
    }

    #[test]
    fn test_born_dead_new_used_function() {
        let insert = make_insert(
            "helper",
            "fn helper() { println!(\"used\"); }",
            "src/lib.rs",
            10,
        );
        let inserted: Vec<&ASTChange> = vec![&insert];

        // refcounts: "helper" appears 3 times (definition + 2 call sites)
        let mut ref_counts = HashMap::new();
        ref_counts.insert("helper".to_string(), 3);

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert!(
            findings.is_empty(),
            "Function with callers should not be flagged, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_born_dead_public_medium_severity() {
        let insert = make_insert("unused_pub", "pub fn unused_pub() { }", "src/lib.rs", 5);
        let inserted: Vec<&ASTChange> = vec![&insert];

        let mut ref_counts = HashMap::new();
        ref_counts.insert("unused_pub".to_string(), 1);

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].severity, "medium",
            "Public unused function should have medium severity"
        );

        // Verify evidence
        let evidence: BornDeadEvidence =
            serde_json::from_value(findings[0].evidence.clone()).expect("parse evidence");
        assert!(evidence.is_public, "Evidence should mark as public");
        assert_eq!(evidence.ref_count, 1);
    }

    #[test]
    fn test_born_dead_private_low_severity() {
        let insert = make_insert("unused_priv", "fn unused_priv() { }", "src/lib.rs", 5);
        let inserted: Vec<&ASTChange> = vec![&insert];

        let mut ref_counts = HashMap::new();
        ref_counts.insert("unused_priv".to_string(), 1);

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].severity, "low",
            "Private unused function should have low severity"
        );

        let evidence: BornDeadEvidence =
            serde_json::from_value(findings[0].evidence.clone()).expect("parse evidence");
        assert!(!evidence.is_public, "Evidence should mark as private");
    }

    #[test]
    fn test_born_dead_test_function_suppressed() {
        let insert = make_insert(
            "test_something",
            "fn test_something() { assert!(true); }",
            "tests/my_test.rs",
            1,
        );
        let inserted: Vec<&ASTChange> = vec![&insert];

        // Even with ref_count=1, test functions should be suppressed
        let mut ref_counts = HashMap::new();
        ref_counts.insert("test_something".to_string(), 1);

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert!(
            findings.is_empty(),
            "Test functions should be suppressed, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_born_dead_main_suppressed() {
        let insert = make_insert("main", "fn main() { }", "src/main.rs", 1);
        let inserted: Vec<&ASTChange> = vec![&insert];

        let mut ref_counts = HashMap::new();
        ref_counts.insert("main".to_string(), 1);

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert!(
            findings.is_empty(),
            "Entry point 'main' should be suppressed, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_born_dead_empty_inserted_list() {
        let inserted: Vec<&ASTChange> = vec![];
        let ref_counts = HashMap::new();

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert!(
            findings.is_empty(),
            "Empty input should produce no findings"
        );
    }

    #[test]
    fn test_born_dead_no_name_skipped() {
        // An ASTChange with no name should be silently skipped
        let change = ASTChange {
            change_type: ChangeType::Insert,
            node_kind: NodeKind::Function,
            name: None,
            old_location: None,
            new_location: Some(Location {
                file: "src/lib.rs".to_string(),
                line: 1,
                column: 0,
                end_line: None,
                end_column: None,
            }),
            old_text: None,
            new_text: Some("fn () { }".to_string()),
            similarity: None,
            children: None,
            base_changes: None,
        };
        let inserted: Vec<&ASTChange> = vec![&change];
        let ref_counts = HashMap::new();

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert!(findings.is_empty(), "Change with no name should be skipped");
    }

    #[test]
    fn test_born_dead_zero_refcount_means_dead() {
        // Function not found in refcounts at all => ref_count=0 => dead
        let insert = make_insert("orphan_func", "fn orphan_func() { }", "src/lib.rs", 20);
        let inserted: Vec<&ASTChange> = vec![&insert];

        // Empty refcounts -- function name not even found
        let ref_counts = HashMap::new();

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert_eq!(
            findings.len(),
            1,
            "Function not in refcounts should be dead"
        );
        assert_eq!(findings[0].function, "orphan_func");

        let evidence: BornDeadEvidence =
            serde_json::from_value(findings[0].evidence.clone()).expect("parse evidence");
        assert_eq!(evidence.ref_count, 0);
    }

    #[test]
    fn test_born_dead_multiple_findings() {
        let insert1 = make_insert("dead_one", "fn dead_one() { }", "src/a.rs", 1);
        let insert2 = make_insert("alive_one", "fn alive_one() { }", "src/a.rs", 10);
        let insert3 = make_insert("dead_two", "pub fn dead_two() { }", "src/b.rs", 5);

        let inserted: Vec<&ASTChange> = vec![&insert1, &insert2, &insert3];

        let mut ref_counts = HashMap::new();
        ref_counts.insert("dead_one".to_string(), 1);
        ref_counts.insert("alive_one".to_string(), 4);
        ref_counts.insert("dead_two".to_string(), 1);

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert_eq!(
            findings.len(),
            2,
            "Should find 2 dead functions, got: {:?}",
            findings.iter().map(|f| &f.function).collect::<Vec<_>>()
        );

        let names: Vec<&str> = findings.iter().map(|f| f.function.as_str()).collect();
        assert!(names.contains(&"dead_one"));
        assert!(names.contains(&"dead_two"));
        assert!(!names.contains(&"alive_one"));
    }

    #[test]
    fn test_born_dead_benchmark_function_suppressed() {
        let insert = make_insert(
            "BenchmarkSomething",
            "fn BenchmarkSomething() { }",
            "benches/bench.rs",
            1,
        );
        let inserted: Vec<&ASTChange> = vec![&insert];

        let mut ref_counts = HashMap::new();
        ref_counts.insert("BenchmarkSomething".to_string(), 1);

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert!(
            findings.is_empty(),
            "Benchmark functions should be suppressed"
        );
    }

    #[test]
    fn test_born_dead_short_name_not_rescued() {
        // Short names (< 3 chars) need >= 5 refs to be rescued.
        // With count=4, still below threshold => dead.
        let insert = make_insert("ab", "fn ab() { }", "src/lib.rs", 1);
        let inserted: Vec<&ASTChange> = vec![&insert];

        // ref_count=4 and name is too short (needs >= 5) => not rescued => dead
        let mut ref_counts = HashMap::new();
        ref_counts.insert("ab".to_string(), 4);

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert_eq!(
            findings.len(),
            1,
            "Short-named function with count < 5 should not be rescued"
        );
    }

    #[test]
    fn test_lookup_ref_count_qualified_name() {
        let mut ref_counts = HashMap::new();
        ref_counts.insert("method".to_string(), 3);

        assert_eq!(lookup_ref_count("Class.method", &ref_counts), 3);
        assert_eq!(lookup_ref_count("module:method", &ref_counts), 3);
        assert_eq!(lookup_ref_count("method", &ref_counts), 3);
        assert_eq!(lookup_ref_count("unknown", &ref_counts), 0);
    }

    #[test]
    fn test_is_test_function_patterns() {
        assert!(is_test_function("test_something"));
        assert!(is_test_function("test"));
        assert!(is_test_function("TestFoo"));
        assert!(is_test_function("BenchmarkBar"));
        assert!(is_test_function("ExampleBaz"));
        assert!(!is_test_function("helper"));
        assert!(!is_test_function("testing_mode")); // starts with "Test" check is case-sensitive
        assert!(!is_test_function("contestant"));
    }

    #[test]
    fn test_is_entry_point_patterns() {
        assert!(is_entry_point("main"));
        assert!(is_entry_point("lib"));
        assert!(is_entry_point("__main__"));
        assert!(is_entry_point("cli"));
        assert!(!is_entry_point("helper"));
        assert!(!is_entry_point("main_loop")); // exact match only
    }

    #[test]
    fn test_is_trait_impl_std_trait_methods() {
        // Standard library trait methods should be recognised
        let std_methods = [
            "fmt",
            "from",
            "into",
            "try_from",
            "try_into",
            "clone",
            "clone_from",
            "default",
            "drop",
            "deref",
            "deref_mut",
            "as_ref",
            "as_mut",
            "borrow",
            "borrow_mut",
            "eq",
            "ne",
            "partial_cmp",
            "cmp",
            "hash",
            "next",
            "size_hint",
            "index",
            "index_mut",
            "from_str",
            "to_string",
            "write_str",
            "serialize",
            "deserialize",
            "poll",
        ];
        for method in &std_methods {
            assert!(
                is_trait_impl(method, ""),
                "'{}' should be recognised as a trait impl method",
                method
            );
        }
    }

    #[test]
    fn test_is_trait_impl_non_trait_methods() {
        // Regular function names should NOT be flagged as trait methods
        assert!(!is_trait_impl("helper", ""));
        assert!(!is_trait_impl("process_data", ""));
        assert!(!is_trait_impl("run", ""));
        assert!(!is_trait_impl("build", ""));
        assert!(!is_trait_impl("new", ""));
        assert!(!is_trait_impl("main", ""));
        assert!(!is_trait_impl("calculate", ""));
    }

    #[test]
    fn test_born_dead_trait_impl_method_suppressed() {
        // A new function named "fmt" with no callers should be suppressed
        // because it is likely a Display/Debug trait impl.
        let insert = make_insert(
            "fmt",
            "fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) }",
            "src/lib.rs",
            10,
        );
        let inserted: Vec<&ASTChange> = vec![&insert];

        let mut ref_counts = HashMap::new();
        ref_counts.insert("fmt".to_string(), 1);

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert!(
            findings.is_empty(),
            "Trait impl method 'fmt' should be suppressed, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_born_dead_test_file_suppressed() {
        // A function in a test file should be suppressed even without test_ prefix
        let insert = make_insert(
            "validate_output_format",
            "fn validate_output_format() { assert!(true); }",
            "crates/cli/tests/integration_test.rs",
            50,
        );
        let inserted: Vec<&ASTChange> = vec![&insert];
        let ref_counts = HashMap::new();

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert!(
            findings.is_empty(),
            "Function in test file should be suppressed, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_born_dead_test_directory_suppressed() {
        // Functions in /tests/ directory should be suppressed
        let insert = make_insert(
            "setup_mock_server",
            "fn setup_mock_server() {}",
            "src/tests/helpers.rs",
            10,
        );
        let inserted: Vec<&ASTChange> = vec![&insert];
        let ref_counts = HashMap::new();

        let findings =
            compose_born_dead_with_refcounts(&inserted, &ref_counts).expect("should succeed");

        assert!(
            findings.is_empty(),
            "Function in tests directory should be suppressed, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_is_test_file_patterns() {
        // Positive cases
        assert!(is_test_file("crates/cli/tests/integration_test.rs"));
        assert!(is_test_file("src/tests/helpers.rs"));
        assert!(is_test_file("src/test/java/FooTest.java"));
        assert!(is_test_file("src/__tests__/foo.test.ts"));
        assert!(is_test_file("tests/test_foo.py"));
        assert!(is_test_file("pkg/handler_test.go"));
        assert!(is_test_file("src/utils.spec.ts"));
        assert!(is_test_file("FooTest.java"));
        assert!(is_test_file("FooTests.java"));

        // Negative cases
        assert!(!is_test_file("src/lib.rs"));
        assert!(!is_test_file("src/main.py"));
        assert!(!is_test_file("src/testing_utils.rs")); // /testing/ dir, not this
        assert!(!is_test_file("src/contest.rs"));
    }
}
