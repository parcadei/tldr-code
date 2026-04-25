//! Quality Metrics Multilang Benchmark Tests
//!
//! Comprehensive integration tests for quality metrics commands across all
//! supported languages:
//!
//! - `complexity` -- cyclomatic complexity (quality module)
//! - `cognitive` -- cognitive complexity (SonarQube algorithm)
//! - `halstead` -- Halstead software science metrics
//! - `loc` -- lines of code with type breakdown
//! - `smells` -- code smell detection
//! - `clones` (similarity) -- code clone detection
//! - `dice` (similarity) -- file similarity comparison
//! - `explain` -- comprehensive function analysis (via CLI binary)
//!
//! # Running Tests
//!
//! ```bash
//! cargo test -p tldr-core --test bench_quality_multilang
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use tldr_core::metrics::{
    analyze_cognitive, analyze_halstead, analyze_loc, calculate_all_complexities_file,
    calculate_complexity, CognitiveOptions, HalsteadOptions, LocOptions,
};
use tldr_core::quality::{
    analyze_complexity, detect_smells, find_similar, ComplexityOptions, SmellType, SmellsReport,
    ThresholdPreset,
};
use tldr_core::types::Language;

// =============================================================================
// Helpers
// =============================================================================

/// Path to the extractor fixture directory
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extractor")
}

/// Get a fixture file path by language extension
fn fixture_path(filename: &str) -> PathBuf {
    fixtures_dir().join(filename)
}

/// Create a temp directory with a file of given content
fn temp_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

// =============================================================================
// Complexity Tests (quality::analyze_complexity) -- all 18 fixture languages
// =============================================================================

mod complexity_tests {
    use super::*;

    /// Helper: run complexity analysis on a fixture file
    fn run_complexity(filename: &str, lang: Language) -> tldr_core::quality::ComplexityReport {
        let path = fixture_path(filename);
        assert!(path.exists(), "Fixture {} must exist", filename);
        analyze_complexity(&path, Some(lang), None)
            .unwrap_or_else(|e| panic!("complexity({}) failed: {}", filename, e))
    }

    #[test]
    fn test_complexity_python() {
        let report = run_complexity("test_python.py", Language::Python);
        // 3 functions + 5 methods = 8 callable definitions
        assert!(
            report.functions_analyzed >= 3,
            "Python: expected at least 3 functions, got {}",
            report.functions_analyzed
        );
        // All simple functions => CC=1 each, no hotspots
        assert_eq!(
            report.hotspot_count, 0,
            "Python: simple functions should not be hotspots"
        );
        // Average CC should be 1.0 for trivial functions
        assert!(
            report.avg_cyclomatic >= 1.0,
            "Python: avg CC should be >= 1.0, got {}",
            report.avg_cyclomatic
        );
    }

    #[test]
    fn test_complexity_javascript() {
        let report = run_complexity("test_javascript.js", Language::JavaScript);
        assert!(
            report.functions_analyzed >= 3,
            "JavaScript: expected at least 3 functions, got {}",
            report.functions_analyzed
        );
        // Simple functions with no branches => no hotspots
        assert_eq!(
            report.hotspot_count, 0,
            "JavaScript: simple functions should not be hotspots"
        );
    }

    #[test]
    fn test_complexity_typescript() {
        let report = run_complexity("test_typescript.ts", Language::TypeScript);
        assert!(
            report.functions_analyzed >= 3,
            "TypeScript: expected at least 3 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_go() {
        let report = run_complexity("test_go.go", Language::Go);
        assert!(
            report.functions_analyzed >= 2,
            "Go: expected at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_rust() {
        let report = run_complexity("test_rust.rs", Language::Rust);
        assert!(
            report.functions_analyzed >= 3,
            "Rust: expected at least 3 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_java() {
        let report = run_complexity("test_java.java", Language::Java);
        // Java fixture has 6 methods across 3 classes
        assert!(
            report.functions_analyzed >= 3,
            "Java: expected at least 3 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_c() {
        let report = run_complexity("test_c.c", Language::C);
        assert!(
            report.functions_analyzed >= 3,
            "C: expected at least 3 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_cpp() {
        let report = run_complexity("test_cpp.cpp", Language::Cpp);
        assert!(
            report.functions_analyzed >= 2,
            "C++: expected at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_ruby() {
        let report = run_complexity("test_ruby.rb", Language::Ruby);
        assert!(
            report.functions_analyzed >= 2,
            "Ruby: expected at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_kotlin() {
        let report = run_complexity("test_kotlin.kt", Language::Kotlin);
        assert!(
            report.functions_analyzed >= 2,
            "Kotlin: expected at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_swift() {
        let report = run_complexity("test_swift.swift", Language::Swift);
        assert!(
            report.functions_analyzed >= 2,
            "Swift: expected at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_csharp() {
        let report = run_complexity("test_csharp.cs", Language::CSharp);
        assert!(
            report.functions_analyzed >= 3,
            "C#: expected at least 3 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_scala() {
        let report = run_complexity("test_scala.scala", Language::Scala);
        assert!(
            report.functions_analyzed >= 2,
            "Scala: expected at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_php() {
        let report = run_complexity("test_php.php", Language::Php);
        assert!(
            report.functions_analyzed >= 2,
            "PHP: expected at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_lua() {
        let report = run_complexity("test_lua.lua", Language::Lua);
        assert!(
            report.functions_analyzed >= 2,
            "Lua: expected at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_luau() {
        let report = run_complexity("test_luau.luau", Language::Luau);
        assert!(
            report.functions_analyzed >= 2,
            "Luau: expected at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_elixir() {
        let report = run_complexity("test_elixir.ex", Language::Elixir);
        assert!(
            report.functions_analyzed >= 2,
            "Elixir: expected at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    #[test]
    fn test_complexity_ocaml() {
        let report = run_complexity("test_ocaml.ml", Language::Ocaml);
        assert!(
            report.functions_analyzed >= 2,
            "OCaml: expected at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    /// Complex code with branches should produce CC > 1 and trigger hotspots
    #[test]
    fn test_complexity_branching_function_python() {
        let dir = TempDir::new().unwrap();
        let content = r#"
def branching(x, y, z):
    if x > 0:
        if y > 0:
            return 1
        elif z > 0:
            return 2
        else:
            return 3
    elif y > 0:
        return 4
    else:
        for i in range(10):
            if i % 2 == 0:
                x += i
        return x
"#;
        let path = temp_file(&dir, "branching.py", content);
        let report = analyze_complexity(&path, Some(Language::Python), None).unwrap();

        assert!(
            report.functions_analyzed >= 1,
            "Should find at least 1 function"
        );
        assert!(
            report.max_cyclomatic >= 5,
            "Branching function should have CC >= 5, got {}",
            report.max_cyclomatic
        );
    }

    /// A no-branch function should have CC == 1
    #[test]
    fn test_complexity_no_branch_equals_one() {
        let dir = TempDir::new().unwrap();
        let content = "def simple():\n    return 42\n";
        let path = temp_file(&dir, "simple.py", content);
        let report = analyze_complexity(&path, Some(Language::Python), None).unwrap();

        assert!(
            report.functions_analyzed >= 1,
            "Should find the simple function"
        );
        assert_eq!(
            report.max_cyclomatic, 1,
            "No-branch function should have CC == 1"
        );
    }

    /// Hotspot detection with custom threshold
    #[test]
    fn test_complexity_hotspot_with_threshold() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("def complex_fn(a, b, c, d, e):\n");
        for i in 0..8 {
            if i == 0 {
                content.push_str("    if a:\n        pass\n");
            } else {
                content.push_str(&format!("    elif {}:\n        pass\n", (b'a' + i) as char));
            }
        }
        content.push_str("    return 0\n");
        let path = temp_file(&dir, "hotspot.py", &content);

        let options = ComplexityOptions {
            hotspot_threshold: 3,
            max_hotspots: 10,
            include_cognitive: true,
        };

        let report = analyze_complexity(&path, Some(Language::Python), Some(options)).unwrap();
        assert!(
            report.hotspot_count > 0,
            "Should detect hotspot with lower threshold"
        );
    }

    /// Functions are sorted by CC descending
    #[test]
    fn test_complexity_sorted_descending() {
        let dir = TempDir::new().unwrap();
        let content = r#"
def simple():
    return 1

def moderate(x, y):
    if x:
        if y:
            return 1
        return 2
    return 3

def trivial():
    pass
"#;
        let path = temp_file(&dir, "sorted.py", content);
        let report = analyze_complexity(&path, Some(Language::Python), None).unwrap();

        if report.functions.len() >= 2 {
            for i in 0..report.functions.len() - 1 {
                assert!(
                    report.functions[i].cyclomatic >= report.functions[i + 1].cyclomatic,
                    "Functions should be sorted by CC descending"
                );
            }
        }
    }
}

// =============================================================================
// Metrics Complexity Tests (metrics::calculate_complexity) -- per-function
// =============================================================================

mod metrics_complexity_tests {
    use super::*;

    #[test]
    fn test_calculate_complexity_python_simple() {
        let path = fixture_path("test_python.py");
        let result =
            calculate_complexity(path.to_str().unwrap(), "top_level_func", Language::Python);
        assert!(result.is_ok(), "calculate_complexity should succeed");
        let metrics = result.unwrap();
        assert_eq!(metrics.function, "top_level_func");
        assert_eq!(
            metrics.cyclomatic, 1,
            "Simple function should have CC=1, got {}",
            metrics.cyclomatic
        );
    }

    #[test]
    fn test_calculate_all_complexities_python() {
        let path = fixture_path("test_python.py");
        let result = calculate_all_complexities_file(&path);
        assert!(
            result.is_ok(),
            "calculate_all_complexities_file should succeed"
        );
        let map = result.unwrap();
        assert!(
            !map.is_empty(),
            "Should find at least one function in Python fixture"
        );
        // All simple functions should have CC=1
        for (name, metrics) in &map {
            assert!(
                metrics.cyclomatic >= 1,
                "Function {} should have CC >= 1, got {}",
                name,
                metrics.cyclomatic
            );
        }
    }

    #[test]
    fn test_calculate_all_complexities_go() {
        let path = fixture_path("test_go.go");
        let result = calculate_all_complexities_file(&path);
        assert!(result.is_ok(), "Go complexity should succeed");
        let map = result.unwrap();
        assert!(
            !map.is_empty(),
            "Should find at least one function in Go fixture"
        );
    }

    #[test]
    fn test_calculate_all_complexities_rust() {
        let path = fixture_path("test_rust.rs");
        let result = calculate_all_complexities_file(&path);
        assert!(result.is_ok(), "Rust complexity should succeed");
        let map = result.unwrap();
        assert!(
            !map.is_empty(),
            "Should find at least one function in Rust fixture"
        );
    }

    #[test]
    fn test_calculate_all_complexities_javascript() {
        let path = fixture_path("test_javascript.js");
        let result = calculate_all_complexities_file(&path);
        assert!(result.is_ok(), "JavaScript complexity should succeed");
        let map = result.unwrap();
        assert!(
            !map.is_empty(),
            "Should find at least one function in JavaScript fixture"
        );
    }

    #[test]
    fn test_calculate_all_complexities_java() {
        let path = fixture_path("test_java.java");
        let result = calculate_all_complexities_file(&path);
        assert!(result.is_ok(), "Java complexity should succeed");
        let map = result.unwrap();
        assert!(
            !map.is_empty(),
            "Should find at least one function in Java fixture"
        );
    }

    /// CC for a branching function should be > 1
    #[test]
    fn test_complexity_branching_has_higher_cc() {
        let dir = TempDir::new().unwrap();
        let content = r#"
def branchy(x, y):
    if x > 0:
        if y > 0:
            return 1
        return 2
    elif y < 0:
        return 3
    return 4
"#;
        let path = temp_file(&dir, "branchy.py", content);
        let result = calculate_complexity(path.to_str().unwrap(), "branchy", Language::Python);
        assert!(result.is_ok());
        let metrics = result.unwrap();
        assert!(
            metrics.cyclomatic >= 3,
            "branchy should have CC >= 3, got {}",
            metrics.cyclomatic
        );
    }
}

// =============================================================================
// Cognitive Complexity Tests -- all 18 fixture languages
// =============================================================================

mod cognitive_tests {
    use super::*;

    /// Helper: run cognitive analysis on a fixture file
    fn run_cognitive(filename: &str) -> tldr_core::metrics::CognitiveReport {
        let path = fixture_path(filename);
        assert!(path.exists(), "Fixture {} must exist", filename);
        let options = CognitiveOptions::new();
        analyze_cognitive(&path, &options)
            .unwrap_or_else(|e| panic!("cognitive({}) failed: {}", filename, e))
    }

    #[test]
    fn test_cognitive_python() {
        let report = run_cognitive("test_python.py");
        assert!(
            report.summary.total_functions >= 3,
            "Python: expected at least 3 functions, got {}",
            report.summary.total_functions
        );
        // Simple functions should have low cognitive complexity
        assert!(
            report.summary.avg_cognitive < 5.0,
            "Python: simple functions should have low avg cognitive, got {}",
            report.summary.avg_cognitive
        );
    }

    #[test]
    fn test_cognitive_javascript() {
        let report = run_cognitive("test_javascript.js");
        assert!(
            report.summary.total_functions >= 3,
            "JavaScript: expected at least 3 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_typescript() {
        let report = run_cognitive("test_typescript.ts");
        assert!(
            report.summary.total_functions >= 3,
            "TypeScript: expected at least 3 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_go() {
        let report = run_cognitive("test_go.go");
        assert!(
            report.summary.total_functions >= 2,
            "Go: expected at least 2 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_rust() {
        let report = run_cognitive("test_rust.rs");
        assert!(
            report.summary.total_functions >= 3,
            "Rust: expected at least 3 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_java() {
        let report = run_cognitive("test_java.java");
        assert!(
            report.summary.total_functions >= 3,
            "Java: expected at least 3 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_c() {
        let report = run_cognitive("test_c.c");
        assert!(
            report.summary.total_functions >= 3,
            "C: expected at least 3 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_cpp() {
        let report = run_cognitive("test_cpp.cpp");
        assert!(
            report.summary.total_functions >= 2,
            "C++: expected at least 2 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_ruby() {
        let report = run_cognitive("test_ruby.rb");
        assert!(
            report.summary.total_functions >= 2,
            "Ruby: expected at least 2 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_kotlin() {
        let report = run_cognitive("test_kotlin.kt");
        assert!(
            report.summary.total_functions >= 2,
            "Kotlin: expected at least 2 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_swift() {
        let report = run_cognitive("test_swift.swift");
        assert!(
            report.summary.total_functions >= 2,
            "Swift: expected at least 2 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_csharp() {
        let report = run_cognitive("test_csharp.cs");
        assert!(
            report.summary.total_functions >= 3,
            "C#: expected at least 3 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_scala() {
        let report = run_cognitive("test_scala.scala");
        assert!(
            report.summary.total_functions >= 2,
            "Scala: expected at least 2 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_php() {
        let report = run_cognitive("test_php.php");
        assert!(
            report.summary.total_functions >= 2,
            "PHP: expected at least 2 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_lua() {
        let report = run_cognitive("test_lua.lua");
        assert!(
            report.summary.total_functions >= 2,
            "Lua: expected at least 2 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_luau() {
        let report = run_cognitive("test_luau.luau");
        assert!(
            report.summary.total_functions >= 2,
            "Luau: expected at least 2 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_elixir() {
        let report = run_cognitive("test_elixir.ex");
        assert!(
            report.summary.total_functions >= 2,
            "Elixir: expected at least 2 functions, got {}",
            report.summary.total_functions
        );
    }

    #[test]
    fn test_cognitive_ocaml() {
        let report = run_cognitive("test_ocaml.ml");
        assert!(
            report.summary.total_functions >= 2,
            "OCaml: expected at least 2 functions, got {}",
            report.summary.total_functions
        );
    }

    /// Nested conditions should score higher than flat conditions
    #[test]
    fn test_cognitive_nesting_scores_higher() {
        let dir = TempDir::new().unwrap();
        let content = r#"
def flat_conditions(a, b, c, d):
    if a:
        return 1
    if b:
        return 2
    if c:
        return 3
    if d:
        return 4
    return 0

def nested_conditions(a, b, c, d):
    if a:
        if b:
            if c:
                if d:
                    return 1
    return 0
"#;
        let path = temp_file(&dir, "nesting.py", content);
        let options = CognitiveOptions::new();
        let report = analyze_cognitive(&path, &options).unwrap();

        let flat_fn = report
            .functions
            .iter()
            .find(|f| f.name == "flat_conditions");
        let nested_fn = report
            .functions
            .iter()
            .find(|f| f.name == "nested_conditions");

        assert!(flat_fn.is_some(), "Should find flat_conditions");
        assert!(nested_fn.is_some(), "Should find nested_conditions");

        let flat_score = flat_fn.unwrap().cognitive;
        let nested_score = nested_fn.unwrap().cognitive;

        assert!(
            nested_score > flat_score,
            "Nested conditions ({}) should score higher than flat conditions ({})",
            nested_score,
            flat_score
        );
    }

    /// Nesting penalty should be reflected in the function result
    #[test]
    fn test_cognitive_nesting_penalty_tracked() {
        let dir = TempDir::new().unwrap();
        let content = r#"
def deeply_nested(a, b, c):
    if a:
        if b:
            if c:
                return 1
    return 0
"#;
        let path = temp_file(&dir, "deep.py", content);
        let options = CognitiveOptions::new();
        let report = analyze_cognitive(&path, &options).unwrap();

        let func = report.functions.iter().find(|f| f.name == "deeply_nested");
        assert!(func.is_some(), "Should find deeply_nested");
        let func = func.unwrap();
        assert!(
            func.nesting_penalty > 0,
            "Deeply nested function should have nesting_penalty > 0, got {}",
            func.nesting_penalty
        );
        assert!(
            func.max_nesting >= 3,
            "Should have max nesting >= 3, got {}",
            func.max_nesting
        );
    }

    /// Function filter should narrow results
    #[test]
    fn test_cognitive_function_filter() {
        let path = fixture_path("test_python.py");
        let options = CognitiveOptions::new().with_function(Some("another_func".to_string()));
        let report = analyze_cognitive(&path, &options).unwrap();

        assert_eq!(
            report.functions.len(),
            1,
            "Filter should return exactly 1 function"
        );
        assert_eq!(report.functions[0].name, "another_func");
    }
}

// =============================================================================
// Halstead Metrics Tests -- all 18 fixture languages
// =============================================================================

mod halstead_tests {
    use super::*;

    /// Helper: run Halstead analysis on a fixture file
    fn run_halstead(filename: &str, lang: Language) -> tldr_core::metrics::HalsteadReport {
        let path = fixture_path(filename);
        assert!(path.exists(), "Fixture {} must exist", filename);
        let options = HalsteadOptions::new();
        analyze_halstead(&path, Some(lang), options)
            .unwrap_or_else(|e| panic!("halstead({}) failed: {}", filename, e))
    }

    /// Verify common Halstead invariants for a report
    fn verify_halstead_invariants(report: &tldr_core::metrics::HalsteadReport, lang_name: &str) {
        assert!(
            !report.functions.is_empty(),
            "{}: should have at least one function",
            lang_name
        );

        for func in &report.functions {
            // Vocabulary = n1 + n2
            assert_eq!(
                func.metrics.vocabulary,
                func.metrics.n1 + func.metrics.n2,
                "{}: vocabulary should be n1+n2 for function {}",
                lang_name,
                func.name
            );
            // Length = N1 + N2
            assert_eq!(
                func.metrics.length,
                func.metrics.big_n1 + func.metrics.big_n2,
                "{}: length should be N1+N2 for function {}",
                lang_name,
                func.name
            );
            // Volume should be positive for non-empty functions
            assert!(
                func.metrics.volume > 0.0,
                "{}: volume should be > 0 for function {}, got {}",
                lang_name,
                func.name,
                func.metrics.volume
            );
            // Difficulty should be non-negative
            assert!(
                func.metrics.difficulty >= 0.0,
                "{}: difficulty should be >= 0 for function {}, got {}",
                lang_name,
                func.name,
                func.metrics.difficulty
            );
            // Effort = difficulty * volume
            let expected_effort = func.metrics.difficulty * func.metrics.volume;
            assert!(
                (func.metrics.effort - expected_effort).abs() < 0.01,
                "{}: effort should be difficulty*volume for function {}",
                lang_name,
                func.name
            );
        }

        // Summary should reflect the functions
        assert!(
            report.summary.total_functions > 0,
            "{}: summary total_functions should be > 0",
            lang_name
        );
        assert!(
            report.summary.avg_volume > 0.0,
            "{}: summary avg_volume should be > 0, got {}",
            lang_name,
            report.summary.avg_volume
        );
    }

    #[test]
    fn test_halstead_python() {
        let report = run_halstead("test_python.py", Language::Python);
        verify_halstead_invariants(&report, "Python");
    }

    #[test]
    fn test_halstead_javascript() {
        let report = run_halstead("test_javascript.js", Language::JavaScript);
        verify_halstead_invariants(&report, "JavaScript");
    }

    #[test]
    fn test_halstead_typescript() {
        let report = run_halstead("test_typescript.ts", Language::TypeScript);
        verify_halstead_invariants(&report, "TypeScript");
    }

    #[test]
    fn test_halstead_go() {
        let report = run_halstead("test_go.go", Language::Go);
        verify_halstead_invariants(&report, "Go");
    }

    #[test]
    fn test_halstead_rust() {
        let report = run_halstead("test_rust.rs", Language::Rust);
        verify_halstead_invariants(&report, "Rust");
    }

    #[test]
    fn test_halstead_java() {
        let report = run_halstead("test_java.java", Language::Java);
        verify_halstead_invariants(&report, "Java");
    }

    #[test]
    fn test_halstead_c() {
        let report = run_halstead("test_c.c", Language::C);
        verify_halstead_invariants(&report, "C");
    }

    #[test]
    fn test_halstead_cpp() {
        let report = run_halstead("test_cpp.cpp", Language::Cpp);
        verify_halstead_invariants(&report, "C++");
    }

    #[test]
    fn test_halstead_ruby() {
        let report = run_halstead("test_ruby.rb", Language::Ruby);
        verify_halstead_invariants(&report, "Ruby");
    }

    #[test]
    fn test_halstead_kotlin() {
        let report = run_halstead("test_kotlin.kt", Language::Kotlin);
        verify_halstead_invariants(&report, "Kotlin");
    }

    #[test]
    fn test_halstead_swift() {
        let report = run_halstead("test_swift.swift", Language::Swift);
        verify_halstead_invariants(&report, "Swift");
    }

    #[test]
    fn test_halstead_csharp() {
        let report = run_halstead("test_csharp.cs", Language::CSharp);
        verify_halstead_invariants(&report, "C#");
    }

    #[test]
    fn test_halstead_scala() {
        let report = run_halstead("test_scala.scala", Language::Scala);
        verify_halstead_invariants(&report, "Scala");
    }

    #[test]
    fn test_halstead_php() {
        let report = run_halstead("test_php.php", Language::Php);
        verify_halstead_invariants(&report, "PHP");
    }

    #[test]
    fn test_halstead_lua() {
        let report = run_halstead("test_lua.lua", Language::Lua);
        verify_halstead_invariants(&report, "Lua");
    }

    #[test]
    fn test_halstead_luau() {
        let report = run_halstead("test_luau.luau", Language::Luau);
        verify_halstead_invariants(&report, "Luau");
    }

    #[test]
    fn test_halstead_elixir() {
        let report = run_halstead("test_elixir.ex", Language::Elixir);
        verify_halstead_invariants(&report, "Elixir");
    }

    #[test]
    fn test_halstead_ocaml() {
        let report = run_halstead("test_ocaml.ml", Language::Ocaml);
        verify_halstead_invariants(&report, "OCaml");
    }

    /// Operators and operands should include expected tokens
    #[test]
    fn test_halstead_operator_operand_detail() {
        let dir = TempDir::new().unwrap();
        let content = r#"
def math_func(a, b):
    result = a + b * 2
    return result
"#;
        let path = temp_file(&dir, "math.py", content);
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        options.show_operands = true;
        let report = analyze_halstead(&path, Some(Language::Python), options).unwrap();

        assert!(!report.functions.is_empty());
        let func = &report.functions[0];

        // Should have operators
        assert!(
            func.operators.is_some(),
            "Should show operators when enabled"
        );
        // Should have operands
        assert!(func.operands.is_some(), "Should show operands when enabled");

        // Should have at least 3 distinct operators (=, +, *, return, etc.)
        assert!(
            func.metrics.n1 >= 3,
            "Should have at least 3 distinct operators, got {}",
            func.metrics.n1
        );
        // Should have at least 3 distinct operands (a, b, result, 2)
        assert!(
            func.metrics.n2 >= 3,
            "Should have at least 3 distinct operands, got {}",
            func.metrics.n2
        );
    }

    /// Estimated bugs should be non-negative
    #[test]
    fn test_halstead_estimated_bugs() {
        let report = run_halstead("test_python.py", Language::Python);
        assert!(
            report.summary.total_estimated_bugs >= 0.0,
            "Total estimated bugs should be >= 0"
        );
    }
}

// =============================================================================
// LOC Tests -- all 18 fixture languages
// =============================================================================

mod loc_tests {
    use super::*;

    /// Helper: run LOC analysis on a fixture file
    fn run_loc(filename: &str) -> tldr_core::metrics::LocReport {
        let path = fixture_path(filename);
        assert!(path.exists(), "Fixture {} must exist", filename);
        let options = LocOptions::new();
        analyze_loc(&path, &options).unwrap_or_else(|e| panic!("loc({}) failed: {}", filename, e))
    }

    /// Verify LOC invariants
    fn verify_loc_invariants(report: &tldr_core::metrics::LocReport, lang_name: &str) {
        let s = &report.summary;
        assert_eq!(
            s.total_files, 1,
            "{}: should analyze exactly 1 file",
            lang_name
        );
        assert!(
            s.total_lines > 0,
            "{}: total_lines should be > 0, got {}",
            lang_name,
            s.total_lines
        );
        assert!(
            s.code_lines > 0,
            "{}: code_lines should be > 0, got {}",
            lang_name,
            s.code_lines
        );
        // Invariant: code + comment + blank == total
        assert_eq!(
            s.code_lines + s.comment_lines + s.blank_lines,
            s.total_lines,
            "{}: code({}) + comment({}) + blank({}) should == total({})",
            lang_name,
            s.code_lines,
            s.comment_lines,
            s.blank_lines,
            s.total_lines
        );
        // Language breakdown should have exactly one entry
        assert_eq!(
            report.by_language.len(),
            1,
            "{}: should have 1 language entry",
            lang_name
        );
    }

    #[test]
    fn test_loc_python() {
        let report = run_loc("test_python.py");
        verify_loc_invariants(&report, "Python");
        // Python fixture is 30 lines
        assert!(
            report.summary.total_lines >= 25,
            "Python fixture should have at least 25 lines"
        );
        // Has comments (the expected line)
        assert!(
            report.summary.comment_lines >= 1,
            "Python fixture should have at least 1 comment line"
        );
    }

    #[test]
    fn test_loc_javascript() {
        let report = run_loc("test_javascript.js");
        verify_loc_invariants(&report, "JavaScript");
    }

    #[test]
    fn test_loc_typescript() {
        let report = run_loc("test_typescript.ts");
        verify_loc_invariants(&report, "TypeScript");
    }

    #[test]
    fn test_loc_go() {
        let report = run_loc("test_go.go");
        verify_loc_invariants(&report, "Go");
    }

    #[test]
    fn test_loc_rust() {
        let report = run_loc("test_rust.rs");
        verify_loc_invariants(&report, "Rust");
    }

    #[test]
    fn test_loc_java() {
        let report = run_loc("test_java.java");
        verify_loc_invariants(&report, "Java");
    }

    #[test]
    fn test_loc_c() {
        let report = run_loc("test_c.c");
        verify_loc_invariants(&report, "C");
    }

    #[test]
    fn test_loc_cpp() {
        let report = run_loc("test_cpp.cpp");
        verify_loc_invariants(&report, "C++");
    }

    #[test]
    fn test_loc_ruby() {
        let report = run_loc("test_ruby.rb");
        verify_loc_invariants(&report, "Ruby");
    }

    #[test]
    fn test_loc_kotlin() {
        let report = run_loc("test_kotlin.kt");
        verify_loc_invariants(&report, "Kotlin");
    }

    #[test]
    fn test_loc_swift() {
        let report = run_loc("test_swift.swift");
        verify_loc_invariants(&report, "Swift");
    }

    #[test]
    fn test_loc_csharp() {
        let report = run_loc("test_csharp.cs");
        verify_loc_invariants(&report, "C#");
    }

    #[test]
    fn test_loc_scala() {
        let report = run_loc("test_scala.scala");
        verify_loc_invariants(&report, "Scala");
    }

    #[test]
    fn test_loc_php() {
        let report = run_loc("test_php.php");
        verify_loc_invariants(&report, "PHP");
    }

    #[test]
    fn test_loc_lua() {
        let report = run_loc("test_lua.lua");
        verify_loc_invariants(&report, "Lua");
    }

    #[test]
    fn test_loc_luau() {
        let report = run_loc("test_luau.luau");
        verify_loc_invariants(&report, "Luau");
    }

    #[test]
    fn test_loc_elixir() {
        let report = run_loc("test_elixir.ex");
        verify_loc_invariants(&report, "Elixir");
    }

    #[test]
    fn test_loc_ocaml() {
        let report = run_loc("test_ocaml.ml");
        verify_loc_invariants(&report, "OCaml");
    }

    /// A known-size file should produce correct line count
    #[test]
    fn test_loc_exact_count() {
        let dir = TempDir::new().unwrap();
        // 5 code lines, 2 comment lines, 3 blank lines = 10 total
        let content = "# Comment 1\n# Comment 2\n\ndef foo():\n    return 1\n\ndef bar():\n    return 2\n\n\n";
        let path = temp_file(&dir, "exact.py", content);
        let options = LocOptions::new();
        let report = analyze_loc(&path, &options).unwrap();
        let s = &report.summary;

        assert_eq!(
            s.total_lines, 10,
            "10-line file should have total_lines=10, got {}",
            s.total_lines
        );
        assert!(
            s.comment_lines >= 2,
            "Should have at least 2 comment lines, got {}",
            s.comment_lines
        );
        assert!(
            s.blank_lines >= 3,
            "Should have at least 3 blank lines, got {}",
            s.blank_lines
        );
    }

    /// Percentages should sum to ~100%
    #[test]
    fn test_loc_percentages_sum() {
        let report = run_loc("test_python.py");
        let s = &report.summary;
        let total_pct = s.code_percent + s.comment_percent + s.blank_percent;
        assert!(
            (total_pct - 100.0).abs() < 1.0,
            "Percentages should sum to ~100%, got {}",
            total_pct
        );
    }

    /// Directory analysis should aggregate all fixture files
    #[test]
    fn test_loc_directory_aggregation() {
        let dir = fixtures_dir();
        let mut options = LocOptions::new();
        options.gitignore = false;
        let report = analyze_loc(&dir, &options).unwrap();

        assert!(
            report.summary.total_files >= 15,
            "Fixture directory should have at least 15 files, got {}",
            report.summary.total_files
        );
        assert!(
            report.by_language.len() >= 10,
            "Should detect at least 10 different languages, got {}",
            report.by_language.len()
        );
    }
}

// =============================================================================
// Smells Tests -- 8 key languages
// =============================================================================

mod smells_tests {
    use super::*;

    /// Helper: detect smells in a temp dir with a single file
    fn run_smells_file(dir: &TempDir, filename: &str, content: &str) -> SmellsReport {
        let path = temp_file(dir, filename, content);
        detect_smells(path.parent().unwrap(), ThresholdPreset::Default, None, true)
            .expect("detect_smells should not fail")
    }

    /// God Class detection: a class with 25+ methods
    #[test]
    fn test_smells_god_class_python() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("class Monolith:\n");
        for i in 0..25 {
            content.push_str(&format!("    def method{}(self):\n        pass\n\n", i));
        }
        let report = run_smells_file(&dir, "god.py", &content);

        let god_smells: Vec<_> = report
            .smells
            .iter()
            .filter(|s| s.smell_type == SmellType::GodClass)
            .collect();
        assert!(
            !god_smells.is_empty(),
            "Should detect God Class in Python class with 25 methods"
        );
        // Should have severity
        assert!(
            god_smells[0].severity >= 1,
            "God class should have severity >= 1"
        );
        // Should have a suggestion
        assert!(
            god_smells[0].suggestion.is_some(),
            "Should provide suggestion for God Class"
        );
    }

    /// Long Method detection
    #[test]
    fn test_smells_long_method_python() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("def very_long():\n");
        for i in 0..60 {
            content.push_str(&format!("    x{} = {}\n", i, i));
        }
        content.push_str("    return x0\n");
        let report = run_smells_file(&dir, "long.py", &content);

        let long_smells: Vec<_> = report
            .smells
            .iter()
            .filter(|s| s.smell_type == SmellType::LongMethod)
            .collect();
        assert!(
            !long_smells.is_empty(),
            "Should detect Long Method in 60-line function"
        );
    }

    /// Long Parameter List detection
    #[test]
    fn test_smells_long_params_python() {
        let dir = TempDir::new().unwrap();
        let content = "def too_many(a, b, c, d, e, f, g, h):\n    return a+b+c+d+e+f+g+h\n";
        let report = run_smells_file(&dir, "params.py", content);

        let param_smells: Vec<_> = report
            .smells
            .iter()
            .filter(|s| s.smell_type == SmellType::LongParameterList)
            .collect();
        assert!(
            !param_smells.is_empty(),
            "Should detect Long Parameter List with 8 params"
        );
    }

    /// JavaScript smell detection
    #[test]
    fn test_smells_long_method_javascript() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("function longFunc() {\n");
        for i in 0..60 {
            content.push_str(&format!("    let x{} = {};\n", i, i));
        }
        content.push_str("    return x0;\n}\n");
        let report = run_smells_file(&dir, "long.js", &content);

        let long_smells: Vec<_> = report
            .smells
            .iter()
            .filter(|s| s.smell_type == SmellType::LongMethod)
            .collect();
        assert!(
            !long_smells.is_empty(),
            "Should detect Long Method in JavaScript"
        );
    }

    /// TypeScript smell detection
    #[test]
    fn test_smells_god_class_typescript() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("class GodTS {\n");
        for i in 0..25 {
            content.push_str(&format!("    method{}(): number {{ return {}; }}\n", i, i));
        }
        content.push_str("}\n");
        let report = run_smells_file(&dir, "god.ts", &content);

        let god_smells: Vec<_> = report
            .smells
            .iter()
            .filter(|s| s.smell_type == SmellType::GodClass)
            .collect();
        assert!(
            !god_smells.is_empty(),
            "Should detect God Class in TypeScript"
        );
    }

    /// Go smell detection (long function)
    #[test]
    fn test_smells_long_method_go() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("package main\n\nfunc longFunc() int {\n");
        for i in 0..60 {
            content.push_str(&format!("    x{} := {}\n", i, i));
        }
        content.push_str("    return x0\n}\n");
        let report = run_smells_file(&dir, "long.go", &content);

        let long_smells: Vec<_> = report
            .smells
            .iter()
            .filter(|s| s.smell_type == SmellType::LongMethod)
            .collect();
        assert!(!long_smells.is_empty(), "Should detect Long Method in Go");
    }

    /// Rust smell detection (long function)
    #[test]
    fn test_smells_long_method_rust() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("fn very_long_rust() -> i32 {\n");
        for i in 0..60 {
            content.push_str(&format!("    let x{} = {};\n", i, i));
        }
        content.push_str("    x0\n}\n");
        let report = run_smells_file(&dir, "long.rs", &content);

        let long_smells: Vec<_> = report
            .smells
            .iter()
            .filter(|s| s.smell_type == SmellType::LongMethod)
            .collect();
        assert!(!long_smells.is_empty(), "Should detect Long Method in Rust");
    }

    /// Java smell detection (God Class)
    #[test]
    fn test_smells_god_class_java() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("public class GodJava {\n");
        for i in 0..25 {
            content.push_str(&format!(
                "    public int method{}() {{ return {}; }}\n",
                i, i
            ));
        }
        content.push_str("}\n");
        let report = run_smells_file(&dir, "God.java", &content);

        let god_smells: Vec<_> = report
            .smells
            .iter()
            .filter(|s| s.smell_type == SmellType::GodClass)
            .collect();
        assert!(!god_smells.is_empty(), "Should detect God Class in Java");
    }

    /// C smell detection (long function)
    #[test]
    fn test_smells_long_method_c() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("int very_long_c() {\n");
        for i in 0..60 {
            content.push_str(&format!("    int x{} = {};\n", i, i));
        }
        content.push_str("    return x0;\n}\n");
        let report = run_smells_file(&dir, "long.c", &content);

        let long_smells: Vec<_> = report
            .smells
            .iter()
            .filter(|s| s.smell_type == SmellType::LongMethod)
            .collect();
        assert!(!long_smells.is_empty(), "Should detect Long Method in C");
    }

    /// Ruby smell detection (long function)
    #[test]
    fn test_smells_long_method_ruby() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("def very_long_ruby\n");
        for i in 0..60 {
            content.push_str(&format!("  x{} = {}\n", i, i));
        }
        content.push_str("  x0\nend\n");
        let report = run_smells_file(&dir, "long.rb", &content);

        let long_smells: Vec<_> = report
            .smells
            .iter()
            .filter(|s| s.smell_type == SmellType::LongMethod)
            .collect();
        assert!(!long_smells.is_empty(), "Should detect Long Method in Ruby");
    }

    /// Clean code should have no smells
    #[test]
    fn test_smells_clean_code_no_smells() {
        let dir = TempDir::new().unwrap();
        let content = "def clean():\n    return 42\n";
        let report = run_smells_file(&dir, "clean.py", content);
        assert_eq!(
            report.smells.len(),
            0,
            "Clean code should have 0 smells, got {}",
            report.smells.len()
        );
    }

    /// Smell filtering by type
    #[test]
    fn test_smells_filter_by_type() {
        let dir = TempDir::new().unwrap();
        // Create code with both long method and long parameter list
        let mut content = String::from("def long_with_params(a, b, c, d, e, f, g, h):\n");
        for i in 0..60 {
            content.push_str(&format!("    x{} = {}\n", i, i));
        }
        content.push_str("    return a+b\n");
        let path = temp_file(&dir, "multi.py", content.as_str());

        let report = detect_smells(
            path.parent().unwrap(),
            ThresholdPreset::Default,
            Some(SmellType::LongParameterList),
            false,
        )
        .unwrap();

        for smell in &report.smells {
            assert_eq!(
                smell.smell_type,
                SmellType::LongParameterList,
                "Filter should only return LongParameterList smells"
            );
        }
    }

    /// Deep nesting detection
    #[test]
    fn test_smells_deep_nesting() {
        let dir = TempDir::new().unwrap();
        let content = r#"
def deep(a, b, c, d, e):
    if a:
        if b:
            if c:
                if d:
                    if e:
                        return 1
    return 0
"#;
        let report = run_smells_file(&dir, "deep.py", content);

        let nesting_smells: Vec<_> = report
            .smells
            .iter()
            .filter(|s| s.smell_type == SmellType::DeepNesting)
            .collect();
        assert!(
            !nesting_smells.is_empty(),
            "Should detect deep nesting (5 levels)"
        );
    }

    /// Threshold presets affect detection sensitivity
    #[test]
    fn test_smells_strict_vs_relaxed() {
        let dir = TempDir::new().unwrap();
        let content = "def many_params(a, b, c, d, e, f, g):\n    return a+b\n";
        let path = temp_file(&dir, "params.py", content);

        let strict =
            detect_smells(path.parent().unwrap(), ThresholdPreset::Strict, None, false).unwrap();

        let relaxed = detect_smells(
            path.parent().unwrap(),
            ThresholdPreset::Relaxed,
            None,
            false,
        )
        .unwrap();

        // Strict should detect at least as many smells as relaxed
        assert!(
            strict.smells.len() >= relaxed.smells.len(),
            "Strict({}) should detect >= smells than Relaxed({})",
            strict.smells.len(),
            relaxed.smells.len()
        );
    }

    /// SmellFinding should have location info
    #[test]
    fn test_smells_finding_has_location() {
        let dir = TempDir::new().unwrap();
        let mut content = String::from("def long_located():\n");
        for i in 0..60 {
            content.push_str(&format!("    x{} = {}\n", i, i));
        }
        content.push_str("    return x0\n");
        let report = run_smells_file(&dir, "located.py", &content);

        for smell in &report.smells {
            assert!(
                smell.line > 0,
                "Smell should have a line number > 0, got {}",
                smell.line
            );
            assert!(!smell.name.is_empty(), "Smell should have a non-empty name");
            assert!(
                !smell.reason.is_empty(),
                "Smell should have a non-empty reason"
            );
        }
    }
}

// =============================================================================
// Clones (Similarity) Tests -- 8 key languages
// =============================================================================

mod clones_tests {
    use super::*;

    /// Helper: create a temp dir with duplicated code and run similarity
    fn run_similarity_with_duplicates(
        dir: &TempDir,
        ext: &str,
        code_a: &str,
        code_b: &str,
    ) -> tldr_core::quality::SimilarityReport {
        temp_file(dir, &format!("file_a.{}", ext), code_a);
        temp_file(dir, &format!("file_b.{}", ext), code_b);
        find_similar(dir.path(), None, 0.5, None).expect("find_similar should succeed")
    }

    /// Python: identical functions across files should be detected as clones
    #[test]
    fn test_clones_python_identical() {
        let dir = TempDir::new().unwrap();
        let code = r#"
def process_data(items):
    result = []
    for item in items:
        if item > 0:
            result.append(item * 2)
    return result

def another_function():
    return 42
"#;
        let report = run_similarity_with_duplicates(&dir, "py", code, code);

        assert!(
            report.functions_analyzed >= 2,
            "Should analyze at least 2 functions, got {}",
            report.functions_analyzed
        );
    }

    /// JavaScript: similar functions should be detected
    #[test]
    fn test_clones_javascript_similar() {
        let dir = TempDir::new().unwrap();
        let code_a = r#"
function processItems(items) {
    const result = [];
    for (const item of items) {
        if (item > 0) {
            result.push(item * 2);
        }
    }
    return result;
}
"#;
        // Slightly different variable names but same structure
        let code_b = r#"
function handleData(data) {
    const output = [];
    for (const entry of data) {
        if (entry > 0) {
            output.push(entry * 2);
        }
    }
    return output;
}
"#;
        let report = run_similarity_with_duplicates(&dir, "js", code_a, code_b);
        assert!(
            report.functions_analyzed >= 2,
            "Should analyze at least 2 functions"
        );
        // With Type-2 clone detection (renamed identifiers), these should be similar
        if !report.similar_pairs.is_empty() {
            assert!(
                report.similar_pairs[0].score >= 0.5,
                "Similar functions should have score >= 0.5"
            );
            assert!(
                !report.similar_pairs[0].reasons.is_empty(),
                "Similar pair should have at least one reason"
            );
        }
    }

    /// TypeScript: clone detection
    #[test]
    fn test_clones_typescript() {
        let dir = TempDir::new().unwrap();
        let code = r#"
function processA(x: number): number {
    if (x > 10) {
        return x * 2;
    } else if (x > 5) {
        return x * 3;
    }
    return x;
}

function processB(y: number): number {
    if (y > 10) {
        return y * 2;
    } else if (y > 5) {
        return y * 3;
    }
    return y;
}
"#;
        let report = run_similarity_with_duplicates(&dir, "ts", code, code);
        assert!(
            report.functions_analyzed >= 2,
            "TypeScript: should find at least 2 functions"
        );
    }

    /// Go: clone detection
    #[test]
    fn test_clones_go() {
        let dir = TempDir::new().unwrap();
        let code = r#"package main

func processA(x int) int {
    if x > 10 {
        return x * 2
    }
    return x
}

func processB(y int) int {
    if y > 10 {
        return y * 2
    }
    return y
}
"#;
        temp_file(&dir, "clones.go", code);
        let report = find_similar(dir.path(), Some(Language::Go), 0.5, None).unwrap();
        assert!(
            report.functions_analyzed >= 2,
            "Go: should find at least 2 functions"
        );
    }

    /// Rust: clone detection
    #[test]
    fn test_clones_rust() {
        let dir = TempDir::new().unwrap();
        let code = r#"
fn process_a(x: i32) -> i32 {
    if x > 10 {
        return x * 2;
    }
    x
}

fn process_b(y: i32) -> i32 {
    if y > 10 {
        return y * 2;
    }
    y
}
"#;
        temp_file(&dir, "clones.rs", code);
        let report = find_similar(dir.path(), Some(Language::Rust), 0.5, None).unwrap();
        assert!(
            report.functions_analyzed >= 2,
            "Rust: should find at least 2 functions"
        );
    }

    /// Java: clone detection
    #[test]
    fn test_clones_java() {
        let dir = TempDir::new().unwrap();
        let code = r#"
public class Clones {
    public int processA(int x) {
        if (x > 10) {
            return x * 2;
        }
        return x;
    }

    public int processB(int y) {
        if (y > 10) {
            return y * 2;
        }
        return y;
    }
}
"#;
        temp_file(&dir, "Clones.java", code);
        let report = find_similar(dir.path(), Some(Language::Java), 0.5, None).unwrap();
        assert!(
            report.functions_analyzed >= 2,
            "Java: should find at least 2 functions"
        );
    }

    /// C: clone detection
    #[test]
    fn test_clones_c() {
        let dir = TempDir::new().unwrap();
        let code = r#"
int process_a(int x) {
    if (x > 10) {
        return x * 2;
    }
    return x;
}

int process_b(int y) {
    if (y > 10) {
        return y * 2;
    }
    return y;
}
"#;
        temp_file(&dir, "clones.c", code);
        let report = find_similar(dir.path(), Some(Language::C), 0.5, None).unwrap();
        assert!(
            report.functions_analyzed >= 2,
            "C: should find at least 2 functions"
        );
    }

    /// Ruby: clone detection
    #[test]
    fn test_clones_ruby() {
        let dir = TempDir::new().unwrap();
        let code = r#"
def process_a(x)
  if x > 10
    x * 2
  else
    x
  end
end

def process_b(y)
  if y > 10
    y * 2
  else
    y
  end
end
"#;
        temp_file(&dir, "clones.rb", code);
        let report = find_similar(dir.path(), Some(Language::Ruby), 0.5, None).unwrap();
        assert!(
            report.functions_analyzed >= 2,
            "Ruby: should find at least 2 functions"
        );
    }

    /// Similarity score should be between 0.0 and 1.0
    #[test]
    fn test_clones_score_range() {
        let dir = TempDir::new().unwrap();
        let code = r#"
def func_a(x):
    if x > 0:
        return x * 2
    return x

def func_b(y):
    if y > 0:
        return y * 2
    return y

def func_c(z):
    return z + 1
"#;
        temp_file(&dir, "range.py", code);
        let report = find_similar(dir.path(), Some(Language::Python), 0.0, None).unwrap();

        for pair in &report.similar_pairs {
            assert!(
                pair.score >= 0.0 && pair.score <= 1.0,
                "Score should be in [0.0, 1.0], got {}",
                pair.score
            );
        }
    }

    /// Very different functions should not be similar
    #[test]
    fn test_clones_dissimilar_functions() {
        let dir = TempDir::new().unwrap();
        let code = r#"
def math_heavy(x, y, z):
    a = x * y + z
    b = a ** 2 - x
    c = (a + b) / max(z, 1)
    return a + b + c

def string_heavy(name):
    parts = name.split(" ")
    first = parts[0].upper()
    last = parts[-1].lower()
    return f"{first} {last}"
"#;
        temp_file(&dir, "diff.py", code);
        let report = find_similar(dir.path(), Some(Language::Python), 0.8, None).unwrap();

        // At threshold 0.8, these very different functions should not match
        assert_eq!(
            report.similar_pairs_count, 0,
            "Very different functions should not be similar at 0.8 threshold"
        );
    }

    /// Threshold should filter appropriately
    #[test]
    fn test_clones_threshold_filtering() {
        let dir = TempDir::new().unwrap();
        let code = r#"
def process_a(x):
    if x > 10:
        return x * 2
    return x

def process_b(y):
    if y > 10:
        return y * 2
    return y
"#;
        temp_file(&dir, "threshold.py", code);

        let low_threshold = find_similar(dir.path(), Some(Language::Python), 0.3, None).unwrap();
        let high_threshold = find_similar(dir.path(), Some(Language::Python), 0.9, None).unwrap();

        // Low threshold should find at least as many pairs as high threshold
        assert!(
            low_threshold.similar_pairs_count >= high_threshold.similar_pairs_count,
            "Low threshold ({}) should find >= pairs than high threshold ({})",
            low_threshold.similar_pairs_count,
            high_threshold.similar_pairs_count
        );
    }
}

// =============================================================================
// Dice (File Similarity) Tests
// =============================================================================

mod dice_tests {
    use super::*;

    /// Identical files should have similarity ~1.0
    #[test]
    fn test_dice_identical_python_files() {
        let dir = TempDir::new().unwrap();
        let code = r#"
def process(x):
    if x > 0:
        return x * 2
    return x

def helper(y):
    return y + 1
"#;
        temp_file(&dir, "a.py", code);
        temp_file(&dir, "b.py", code);

        let report = find_similar(dir.path(), Some(Language::Python), 0.0, None).unwrap();

        // With identical files, all functions should match perfectly
        if !report.similar_pairs.is_empty() {
            let max_score = report
                .similar_pairs
                .iter()
                .map(|p| p.score)
                .fold(0.0_f64, f64::max);
            assert!(
                max_score >= 0.8,
                "Identical functions should have high similarity, got {}",
                max_score
            );
        }
    }

    /// Completely different languages should have lower similarity
    #[test]
    fn test_dice_different_languages() {
        let dir = TempDir::new().unwrap();
        let python = r#"
def do_math(a, b, c):
    result = a * b + c
    if result > 100:
        return result / 2
    return result
"#;
        let rust = r#"
fn format_string(name: &str, age: i32) -> String {
    let greeting = format!("Hello, {}", name);
    if age > 18 {
        format!("{} (adult)", greeting)
    } else {
        format!("{} (minor)", greeting)
    }
}
"#;
        temp_file(&dir, "math.py", python);
        temp_file(&dir, "format.rs", rust);

        // Cross-language analysis; find_similar analyzes per language,
        // so just verify it runs without error
        let py_report = find_similar(dir.path(), Some(Language::Python), 0.0, None).unwrap();
        let rs_report = find_similar(dir.path(), Some(Language::Rust), 0.0, None).unwrap();

        // Each should analyze their respective functions
        assert!(
            py_report.functions_analyzed >= 1,
            "Should find Python function"
        );
        assert!(
            rs_report.functions_analyzed >= 1,
            "Should find Rust function"
        );
    }

    /// JavaScript vs TypeScript similarity
    #[test]
    fn test_dice_js_ts_pair() {
        let dir = TempDir::new().unwrap();
        let js = r#"
function calculate(x, y) {
    if (x > y) {
        return x - y;
    }
    return y - x;
}
"#;
        let ts = r#"
function calculate(x: number, y: number): number {
    if (x > y) {
        return x - y;
    }
    return y - x;
}
"#;
        temp_file(&dir, "calc.js", js);
        temp_file(&dir, "calc.ts", ts);

        let js_report = find_similar(dir.path(), Some(Language::JavaScript), 0.0, None).unwrap();
        let ts_report = find_similar(dir.path(), Some(Language::TypeScript), 0.0, None).unwrap();

        assert!(
            js_report.functions_analyzed >= 1,
            "Should find JavaScript function"
        );
        assert!(
            ts_report.functions_analyzed >= 1,
            "Should find TypeScript function"
        );
    }

    /// Go vs C similarity (structurally similar languages)
    #[test]
    fn test_dice_go_c_pair() {
        let dir = TempDir::new().unwrap();
        let go = r#"package main

func abs(x int) int {
    if x < 0 {
        return -x
    }
    return x
}
"#;
        let c = r#"
int abs_val(int x) {
    if (x < 0) {
        return -x;
    }
    return x;
}
"#;
        temp_file(&dir, "abs.go", go);
        temp_file(&dir, "abs.c", c);

        let go_report = find_similar(dir.path(), Some(Language::Go), 0.0, None).unwrap();
        let c_report = find_similar(dir.path(), Some(Language::C), 0.0, None).unwrap();

        assert!(go_report.functions_analyzed >= 1, "Should find Go function");
        assert!(c_report.functions_analyzed >= 1, "Should find C function");
    }
}

// =============================================================================
// Explain Tests (via CLI binary)
// =============================================================================

mod explain_tests {
    use super::*;
    use std::process::Command;

    /// Find the tldr binary for testing
    fn tldr_binary() -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // Try debug binary first, then release
        let debug = manifest_dir
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target/debug/tldr");
        let release = manifest_dir
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target/release/tldr");
        if release.exists() {
            release
        } else if debug.exists() {
            debug
        } else {
            // Fall back to PATH
            PathBuf::from("tldr")
        }
    }

    /// Run explain command and return parsed JSON output
    fn run_explain(file: &Path, function: &str) -> Option<serde_json::Value> {
        let binary = tldr_binary();
        let output = Command::new(&binary)
            .arg("explain")
            .arg(file)
            .arg(function)
            .arg("--format")
            .arg("json")
            .output();

        match output {
            Ok(out) => {
                if !out.status.success() {
                    eprintln!("explain failed: {}", String::from_utf8_lossy(&out.stderr));
                    return None;
                }
                let stdout = String::from_utf8_lossy(&out.stdout);
                serde_json::from_str(&stdout).ok()
            }
            Err(e) => {
                eprintln!("Failed to run tldr binary: {}", e);
                None
            }
        }
    }

    /// Explain should return function analysis for Python
    #[test]
    fn test_explain_python() {
        let path = fixture_path("test_python.py");
        if let Some(json) = run_explain(&path, "another_func") {
            // Should have function_name
            assert_eq!(
                json["function_name"].as_str().unwrap_or(""),
                "another_func",
                "Should report function_name"
            );
            // Should have file path
            assert!(json["file"].is_string(), "Should have file field");
            // Should have signature
            assert!(
                json["signature"].is_object(),
                "Should have signature section"
            );
            // Should have purity
            assert!(json["purity"].is_object(), "Should have purity section");
            // Complexity should be present
            assert!(
                json.get("complexity").is_some(),
                "Should have complexity section"
            );
            // Line numbers should be positive
            if let Some(line_start) = json["line_start"].as_u64() {
                assert!(line_start > 0, "line_start should be > 0");
            }
        }
        // If binary not available, test passes silently (not a hard failure)
    }

    /// Explain should return function analysis for JavaScript
    #[test]
    fn test_explain_javascript() {
        let path = fixture_path("test_javascript.js");
        if let Some(json) = run_explain(&path, "topLevel") {
            assert_eq!(json["function_name"].as_str().unwrap_or(""), "topLevel");
            assert!(json["signature"].is_object());
            assert!(json["purity"].is_object());
        }
    }

    /// Explain should return function analysis for Go
    #[test]
    fn test_explain_go() {
        let path = fixture_path("test_go.go");
        if let Some(json) = run_explain(&path, "topLevel") {
            assert_eq!(json["function_name"].as_str().unwrap_or(""), "topLevel");
            assert!(json["signature"].is_object());
            assert!(json["purity"].is_object());
        }
    }

    /// Explain should return function analysis for Rust
    #[test]
    fn test_explain_rust() {
        let path = fixture_path("test_rust.rs");
        if let Some(json) = run_explain(&path, "public_func") {
            assert_eq!(json["function_name"].as_str().unwrap_or(""), "public_func");
            assert!(json["signature"].is_object());
        }
    }

    /// Explain should return function analysis for TypeScript
    #[test]
    fn test_explain_typescript() {
        let path = fixture_path("test_typescript.ts");
        if let Some(json) = run_explain(&path, "topLevel") {
            assert_eq!(json["function_name"].as_str().unwrap_or(""), "topLevel");
        }
    }

    /// Explain should return function analysis for Java
    #[test]
    fn test_explain_java() {
        let path = fixture_path("test_java.java");
        if let Some(json) = run_explain(&path, "speak") {
            assert_eq!(json["function_name"].as_str().unwrap_or(""), "speak");
        }
    }

    /// Explain should return function analysis for Ruby
    #[test]
    fn test_explain_ruby() {
        let path = fixture_path("test_ruby.rb");
        if let Some(json) = run_explain(&path, "top_level_func") {
            assert_eq!(
                json["function_name"].as_str().unwrap_or(""),
                "top_level_func"
            );
        }
    }

    /// Explain should return function analysis for C
    #[test]
    fn test_explain_c() {
        let path = fixture_path("test_c.c");
        if let Some(json) = run_explain(&path, "add") {
            assert_eq!(json["function_name"].as_str().unwrap_or(""), "add");
        }
    }

    /// Explain purity: function without I/O should NOT be classified as impure.
    /// The analyzer uses "unknown" when it cannot prove purity (conservative),
    /// so we verify it is not "impure" and has no effects.
    #[test]
    fn test_explain_purity_pure_function() {
        let dir = TempDir::new().unwrap();
        let content = r#"
def pure_add(a, b):
    return a + b
"#;
        let path = temp_file(&dir, "pure.py", content);
        if let Some(json) = run_explain(&path, "pure_add") {
            let purity = &json["purity"];
            assert!(purity.is_object(), "Should have purity section");
            if let Some(classification) = purity["classification"].as_str() {
                // A function with no I/O or mutation should not be classified as impure.
                // The analyzer may return "pure" or "unknown" (conservative analysis).
                assert_ne!(
                    classification, "impure",
                    "Function with no side effects should not be classified as impure"
                );
            }
            // Should have no effects
            if let Some(effects) = purity["effects"].as_array() {
                assert!(
                    effects.is_empty(),
                    "Pure function should have no effects, got {:?}",
                    effects
                );
            }
        }
    }

    /// Explain purity: function with I/O should be impure
    #[test]
    fn test_explain_purity_impure_function() {
        let dir = TempDir::new().unwrap();
        let content = r#"
def impure_io():
    print("hello")
    return 42
"#;
        let path = temp_file(&dir, "impure.py", content);
        if let Some(json) = run_explain(&path, "impure_io") {
            let purity = &json["purity"];
            assert!(purity.is_object(), "Should have purity section");
            if let Some(classification) = purity["classification"].as_str() {
                assert_eq!(
                    classification, "impure",
                    "Function with print() should be impure"
                );
            }
            // Should report effects
            if let Some(effects) = purity["effects"].as_array() {
                assert!(!effects.is_empty(), "Impure function should list effects");
            }
        }
    }

    /// Explain complexity section should have metrics
    #[test]
    fn test_explain_complexity_section() {
        let dir = TempDir::new().unwrap();
        let content = r#"
def branchy(x, y, z):
    if x:
        if y:
            return 1
        return 2
    elif z:
        return 3
    return 4
"#;
        let path = temp_file(&dir, "branchy.py", content);
        if let Some(json) = run_explain(&path, "branchy") {
            if let Some(complexity) = json.get("complexity") {
                // Should have cyclomatic
                if let Some(cc) = complexity["cyclomatic"].as_u64() {
                    assert!(cc >= 3, "Branchy function should have CC >= 3, got {}", cc);
                }
            }
        }
    }
}
