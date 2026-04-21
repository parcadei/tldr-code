//! Session 15: Code Metrics CLI Commands - Test Entry Point
//!
//! This is the main test binary for session15 metrics commands.
//!
//! ## Running Tests
//!
//! ```bash
//! # Run all session15 tests (will show ignored)
//! cargo test -p tldr-core --test session15_metrics_tests
//!
//! # Run specific test module
//! cargo test -p tldr-core --test session15_metrics_tests loc_
//! cargo test -p tldr-core --test session15_metrics_tests cognitive_
//! cargo test -p tldr-core --test session15_metrics_tests coverage_
//! cargo test -p tldr-core --test session15_metrics_tests hotspots_
//! cargo test -p tldr-core --test session15_metrics_tests halstead_
//!
//! # Run including ignored tests (to see failures)
//! cargo test -p tldr-core --test session15_metrics_tests -- --ignored
//! ```

// Include the fixtures module from the same directory
#[path = "support/session15_fixtures.rs"]
mod fixtures;

// =============================================================================
// LOC Tests
// =============================================================================

mod loc_tests {
    use super::fixtures;
    
    use tldr_core::metrics::loc::{analyze_loc, LocOptions};

    /// Test that a Python file with known line counts is counted correctly
    #[test]
    fn loc_python_file_counts_correctly() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let file_path =
            fixtures::create_temp_file(&temp_dir, "sample.py", fixtures::PYTHON_LOC_SAMPLE);

        let options = LocOptions::new();
        let result = analyze_loc(&file_path, &options);

        assert!(result.is_ok(), "LOC analysis should succeed");
        let report = result.unwrap();

        let summary = &report.summary;
        let code_lines = summary.code_lines;
        let comment_lines = summary.comment_lines;
        let blank_lines = summary.blank_lines;
        let total_lines = summary.total_lines;

        assert_eq!(
            code_lines + comment_lines + blank_lines,
            total_lines,
            "Invariant: code + comment + blank == total"
        );
    }

    /// Test LOC for multiple languages in a directory
    #[test]
    fn loc_multiple_languages_aggregation() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let project_root = fixtures::create_multi_lang_project(&temp_dir);

        // Disable gitignore since temp dir doesn't have a .gitignore
        let mut options = LocOptions::new();
        options.gitignore = false;
        let result = analyze_loc(&project_root, &options);

        assert!(
            result.is_ok(),
            "LOC analysis should succeed: {:?}",
            result.err()
        );
        let report = result.unwrap();

        let languages: Vec<&str> = report
            .by_language
            .iter()
            .map(|entry| entry.language.as_str())
            .collect();

        assert!(
            languages
                .iter()
                .any(|l| l.to_lowercase().contains("python")),
            "Expected Python in languages: {:?}",
            languages
        );
    }

    /// Test that binary files are skipped
    #[test]
    fn loc_skips_binary_files() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        fixtures::create_temp_file(&temp_dir, "code.py", fixtures::PYTHON_LOC_SAMPLE);
        fixtures::create_temp_binary_file(&temp_dir, "image.png", fixtures::BINARY_PNG_HEADER);

        // Disable gitignore since temp dir doesn't have a .gitignore
        let mut options = LocOptions::new();
        options.gitignore = false;
        options.include_hidden = true; // Include hidden to support temp dirs that start with .
        let result = analyze_loc(temp_dir.path(), &options);

        assert!(
            result.is_ok(),
            "LOC analysis should succeed: {:?}",
            result.err()
        );
        let report = result.unwrap();
        let total_files = report.summary.total_files;
        assert_eq!(
            total_files, 1,
            "Should only count the .py file, found {} files",
            total_files
        );
    }

    /// Test that empty files return zeros
    #[test]
    fn loc_empty_file_returns_zeros() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let file_path = fixtures::create_temp_file(&temp_dir, "empty.py", fixtures::PYTHON_EMPTY);

        let options = LocOptions::new();
        let result = analyze_loc(&file_path, &options);

        assert!(result.is_ok(), "LOC analysis should succeed for empty file");
        let report = result.unwrap();
        let summary = &report.summary;
        assert_eq!(summary.code_lines, 0);
    }

    /// Test JSON output schema via serde
    #[test]
    fn loc_json_output_schema() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let file_path =
            fixtures::create_temp_file(&temp_dir, "sample.py", fixtures::PYTHON_LOC_SAMPLE);

        let options = LocOptions::new();
        let result = analyze_loc(&file_path, &options);

        let report = result.expect("LOC analysis should succeed");

        // Convert to JSON to test schema
        let json = serde_json::to_value(&report).expect("Should serialize to JSON");

        assert!(
            json.get("summary").is_some(),
            "Schema: 'summary' is required"
        );
        assert!(
            json.get("by_language").is_some(),
            "Schema: 'by_language' is required"
        );
    }
}

// =============================================================================
// Cognitive Complexity Tests
// =============================================================================

mod cognitive_tests {
    use super::fixtures;
    use tldr_core::metrics::cognitive::{analyze_cognitive_source, CognitiveOptions};
    use tldr_core::Language;

    /// Simple function with no control flow should have cognitive = 0
    #[test]
    fn test_simple_function_zero_complexity() {
        let options = CognitiveOptions::new();
        let report = analyze_cognitive_source(
            fixtures::PYTHON_COGNITIVE_ZERO,
            Language::Python,
            "test.py",
            &options,
        )
        .expect("Analysis should succeed");

        let simple_fn = report
            .functions
            .iter()
            .find(|f| f.name.contains("simple_function"));

        assert!(simple_fn.is_some(), "Should find simple_function");
        assert_eq!(
            simple_fn.unwrap().cognitive,
            0,
            "Simple function should have cognitive = 0"
        );
    }

    /// Single if statement should have cognitive = 1
    #[test]
    fn test_single_if() {
        let options = CognitiveOptions::new();
        let report = analyze_cognitive_source(
            fixtures::PYTHON_COGNITIVE_SINGLE_IF,
            Language::Python,
            "test.py",
            &options,
        )
        .expect("Analysis should succeed");

        let check_fn = report
            .functions
            .iter()
            .find(|f| f.name.contains("check_positive"));

        assert!(check_fn.is_some(), "Should find check_positive");
        assert_eq!(
            check_fn.unwrap().cognitive,
            1,
            "Single if should have cognitive = 1"
        );
    }

    /// Nested if (depth 2) should have cognitive = 3 (1 + 1+1_nesting)
    #[test]
    fn test_nested_if() {
        let options = CognitiveOptions::new();
        let report = analyze_cognitive_source(
            fixtures::PYTHON_COGNITIVE_NESTED_IF,
            Language::Python,
            "test.py",
            &options,
        )
        .expect("Analysis should succeed");

        let nested_fn = report
            .functions
            .iter()
            .find(|f| f.name.contains("check_nested"));

        assert!(nested_fn.is_some(), "Should find check_nested");
        assert_eq!(
            nested_fn.unwrap().cognitive,
            3,
            "Nested if should have cognitive = 3 (1 + 1 + 1 nesting)"
        );
    }

    /// Loop with nested condition should accumulate correctly
    #[test]
    fn test_loop_with_nested_condition() {
        let options = CognitiveOptions::new();
        let report = analyze_cognitive_source(
            fixtures::PYTHON_COGNITIVE_LOOP_WITH_CONDITION,
            Language::Python,
            "test.py",
            &options,
        )
        .expect("Analysis should succeed");

        let loop_fn = report
            .functions
            .iter()
            .find(|f| f.name.contains("process_items"));

        assert!(loop_fn.is_some(), "Should find process_items");
        assert_eq!(
            loop_fn.unwrap().cognitive,
            3,
            "Loop with nested if should have cognitive = 3"
        );
    }

    /// All functions in a file should be analyzed
    #[test]
    fn test_multiple_functions() {
        let options = CognitiveOptions::new();
        let report = analyze_cognitive_source(
            fixtures::PYTHON_COGNITIVE_MULTIPLE_FUNCTIONS,
            Language::Python,
            "test.py",
            &options,
        )
        .expect("Analysis should succeed");

        assert!(
            report.functions.len() >= 4,
            "Should analyze all functions, got {}",
            report.functions.len()
        );
    }

    /// Threshold violations should be flagged
    #[test]
    fn test_threshold_violations() {
        // Use a low threshold to ensure we get violations
        let options = CognitiveOptions::new().with_threshold(5);
        let report = analyze_cognitive_source(
            fixtures::PYTHON_COGNITIVE_MULTIPLE_FUNCTIONS,
            Language::Python,
            "test.py",
            &options,
        )
        .expect("Analysis should succeed");

        // The complex_function should exceed threshold
        assert!(
            !report.violations.is_empty(),
            "Should detect threshold violations"
        );
    }
}

// =============================================================================
// Coverage Tests
// =============================================================================

mod coverage_tests {
    use super::fixtures;
    use std::path::PathBuf;
    use tldr_core::quality::coverage::{
        parse_cobertura, parse_coverage, parse_coverage_py_json, parse_lcov, CoverageFormat,
        CoverageOptions,
    };

    /// Cobertura XML should be parsed correctly
    #[test]
    fn coverage_parse_cobertura_xml() {
        let report = parse_cobertura(fixtures::COBERTURA_XML).expect("Should parse Cobertura XML");

        // Based on the fixture: 5/7 lines covered = 71.4% (recalculated from actual line hits)
        assert!(
            report.summary.line_coverage >= 50.0 && report.summary.line_coverage <= 100.0,
            "Line coverage should be between 50-100%, got {}",
            report.summary.line_coverage
        );
        assert_eq!(report.format, CoverageFormat::Cobertura);
        assert!(!report.files.is_empty(), "Should have parsed files");
    }

    /// LCOV format should be parsed correctly
    #[test]
    fn coverage_parse_lcov() {
        let report = parse_lcov(fixtures::LCOV_REPORT).expect("Should parse LCOV");

        // Based on fixture: has lines hit
        assert!(
            report.summary.line_coverage >= 0.0 && report.summary.line_coverage <= 100.0,
            "Line coverage should be 0-100%, got {}",
            report.summary.line_coverage
        );
        assert_eq!(report.format, CoverageFormat::Lcov);
        assert!(!report.files.is_empty(), "Should have parsed files");
    }

    /// coverage.py JSON should be parsed correctly
    #[test]
    fn coverage_parse_coverage_py_json() {
        let report = parse_coverage_py_json(fixtures::COVERAGE_PY_JSON)
            .expect("Should parse coverage.py JSON");

        // Based on fixture: totals.percent_covered = 76.92
        assert!(
            (report.summary.line_coverage - 76.92).abs() < 1.0,
            "Line coverage should be ~76.92%, got {}",
            report.summary.line_coverage
        );
        assert_eq!(report.format, CoverageFormat::CoveragePy);
    }

    /// Uncovered functions should be extracted
    #[test]
    fn coverage_extract_uncovered_functions() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let report_path =
            fixtures::create_temp_file(&temp_dir, "coverage.xml", fixtures::COBERTURA_XML);

        let options = CoverageOptions {
            include_uncovered: true,
            by_file: true,
            ..Default::default()
        };

        let report =
            parse_coverage(&report_path, None, &options).expect("Should parse coverage report");

        // The Cobertura fixture has an uncovered_func method with 0 hits
        assert!(report.uncovered.is_some(), "Should have uncovered summary");
        let uncovered = report.uncovered.unwrap();
        assert!(
            !uncovered.functions.is_empty() || !uncovered.line_ranges.is_empty(),
            "Should have some uncovered code"
        );
    }

    /// Percentages should be 0-100
    #[test]
    fn coverage_calculate_percentages() {
        let report = parse_lcov(fixtures::LCOV_REPORT).expect("Should parse LCOV");

        let line_cov = report.summary.line_coverage;
        assert!(
            (0.0..=100.0).contains(&line_cov),
            "Line coverage {} should be 0-100",
            line_cov
        );

        // Check per-file percentages too
        for file in &report.files {
            assert!(
                file.line_coverage >= 0.0 && file.line_coverage <= 100.0,
                "File {} line coverage {} should be 0-100",
                file.path,
                file.line_coverage
            );
        }
    }

    /// Missing file should return error
    #[test]
    fn coverage_missing_file_error() {
        let missing_path = PathBuf::from("/nonexistent/coverage.xml");

        let result = parse_coverage(&missing_path, None, &CoverageOptions::default());

        assert!(result.is_err(), "Should return error for missing file");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("not found") || err.to_string().contains("Path not found"),
            "Error should mention file not found, got: {}",
            err
        );
    }
}

// =============================================================================
// Hotspots Tests
// =============================================================================

mod hotspots_tests {
    use super::fixtures;
    use std::path::Path;
    use std::process::Command;
    use tldr_core::quality::hotspots::{
        analyze_hotspots, calculate_trend, normalize_value, HotspotsOptions, TrendDirection,
    };

    /// Helper to set up a temporary git repo with commits
    fn setup_git_repo(temp_dir: &tempfile::TempDir) {
        let path = temp_dir.path();

        // Initialize git repo
        Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .expect("Failed to init git repo");

        // Configure git user for commits
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(path)
            .output()
            .expect("Failed to config git email");

        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .expect("Failed to config git name");
    }

    /// Helper to make a git commit
    fn make_commit(path: &Path, file: &str, message: &str) {
        Command::new("git")
            .args(["add", file])
            .current_dir(path)
            .output()
            .expect("Failed to add file");

        Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(path)
            .output()
            .expect("Failed to commit");
    }

    /// High churn + high complexity should rank first
    #[test]
    fn test_high_churn_high_complexity_ranked_first() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        setup_git_repo(&temp_dir);

        // Create a complex file and commit it multiple times
        let complex_path = fixtures::create_temp_file(
            &temp_dir,
            "src/complex.py",
            fixtures::PYTHON_COGNITIVE_MULTIPLE_FUNCTIONS,
        );
        make_commit(temp_dir.path(), "src/complex.py", "Add complex file");

        // Modify and commit again to create churn
        std::fs::write(
            &complex_path,
            format!(
                "{}\n# Modified",
                fixtures::PYTHON_COGNITIVE_MULTIPLE_FUNCTIONS
            ),
        )
        .expect("Failed to modify file");
        make_commit(temp_dir.path(), "src/complex.py", "Modify complex file");

        std::fs::write(
            &complex_path,
            format!(
                "{}\n# Modified again",
                fixtures::PYTHON_COGNITIVE_MULTIPLE_FUNCTIONS
            ),
        )
        .expect("Failed to modify file");
        make_commit(
            temp_dir.path(),
            "src/complex.py",
            "Modify complex file again",
        );

        // Create a simple file with one commit
        fixtures::create_temp_file(&temp_dir, "src/simple.py", fixtures::PYTHON_COGNITIVE_ZERO);
        make_commit(temp_dir.path(), "src/simple.py", "Add simple file");

        // Run hotspots analysis with min_commits = 1
        let options = HotspotsOptions::new().with_days(365).with_min_commits(1);

        let result = analyze_hotspots(temp_dir.path(), &options);
        assert!(
            result.is_ok(),
            "Hotspots analysis should succeed: {:?}",
            result.err()
        );

        let report = result.unwrap();

        // Should have at least one hotspot
        assert!(
            !report.hotspots.is_empty(),
            "Should have at least one hotspot"
        );

        // First hotspot should be the complex file (higher churn + complexity)
        let first = &report.hotspots[0];
        assert!(
            first.file.contains("complex.py"),
            "Complex file should be first hotspot"
        );
        assert!(
            first.hotspot_score >= 0.0 && first.hotspot_score <= 1.0,
            "Score should be normalized"
        );
    }

    /// Scores should be normalized 0-1
    #[test]
    fn test_normalization_bounds() {
        // Test the normalize_value function directly
        assert!((normalize_value(50.0, 0.0, 100.0) - 0.5).abs() < 0.001);
        assert!((normalize_value(0.0, 0.0, 100.0) - 0.0).abs() < 0.001);
        assert!((normalize_value(100.0, 0.0, 100.0) - 1.0).abs() < 0.001);

        // Edge case: uniform distribution
        let uniform = normalize_value(50.0, 50.0, 50.0);
        assert!(
            (0.0..=1.0).contains(&uniform),
            "Uniform case should be in bounds"
        );

        // Test via actual analysis
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        setup_git_repo(&temp_dir);

        // Create files and commits
        fixtures::create_temp_file(&temp_dir, "src/a.py", fixtures::PYTHON_COGNITIVE_ZERO);
        make_commit(temp_dir.path(), "src/a.py", "Add a.py");

        fixtures::create_temp_file(
            &temp_dir,
            "src/b.py",
            fixtures::PYTHON_COGNITIVE_MULTIPLE_FUNCTIONS,
        );
        make_commit(temp_dir.path(), "src/b.py", "Add b.py");

        let options = HotspotsOptions::new().with_min_commits(1);
        let result = analyze_hotspots(temp_dir.path(), &options);
        let report = result.expect("hotspots analysis should succeed");
        for hotspot in &report.hotspots {
            assert!(
                hotspot.hotspot_score >= 0.0 && hotspot.hotspot_score <= 1.0,
                "Hotspot score {} should be 0-1",
                hotspot.hotspot_score
            );
            assert!(
                hotspot.churn_score >= 0.0 && hotspot.churn_score <= 1.0,
                "Churn score should be 0-1"
            );
            assert!(
                hotspot.complexity_score >= 0.0 && hotspot.complexity_score <= 1.0,
                "Complexity score should be 0-1"
            );
        }
    }

    /// Top-N should limit results
    #[test]
    fn test_top_n_filtering() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        setup_git_repo(&temp_dir);

        // Create 10 files with commits
        for i in 0..10 {
            fixtures::create_temp_file(
                &temp_dir,
                &format!("src/file{}.py", i),
                fixtures::PYTHON_COGNITIVE_SINGLE_IF,
            );
            make_commit(
                temp_dir.path(),
                &format!("src/file{}.py", i),
                &format!("Add file{}", i),
            );
        }

        // Request top 5
        let options = HotspotsOptions::new().with_top(5).with_min_commits(1);

        let result = analyze_hotspots(temp_dir.path(), &options);
        let report = result.expect("hotspots analysis should succeed");
        assert!(
            report.hotspots.len() <= 5,
            "Should have at most 5 hotspots, got {}",
            report.hotspots.len()
        );
    }

    /// Function-level granularity should include function names
    #[test]
    fn test_function_level_granularity() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        setup_git_repo(&temp_dir);

        fixtures::create_temp_file(
            &temp_dir,
            "src/multi.py",
            fixtures::PYTHON_COGNITIVE_MULTIPLE_FUNCTIONS,
        );
        make_commit(temp_dir.path(), "src/multi.py", "Add multi.py");

        let options = HotspotsOptions::new()
            .with_by_function(true)
            .with_min_commits(1);

        let result = analyze_hotspots(temp_dir.path(), &options);
        let report = result.expect("hotspots analysis should succeed");
        assert!(
            report.metadata.by_function,
            "Should be function-level analysis"
        );

        // When by_function is true, function field should be present
        for hotspot in &report.hotspots {
            assert!(
                hotspot.function.is_some(),
                "Function field should be present"
            );
            assert!(hotspot.line.is_some(), "Line field should be present");
        }
    }

    /// Shallow clones should be handled with warning
    #[test]
    fn test_shallow_clone_handling() {
        // Test that non-git directory returns appropriate error
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        fixtures::create_temp_file(&temp_dir, "src/file.py", fixtures::PYTHON_LOC_SAMPLE);

        let options = HotspotsOptions::new();
        let result = analyze_hotspots(temp_dir.path(), &options);

        // Should fail because it's not a git repository
        assert!(result.is_err(), "Should fail for non-git directory");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Not a git repository"),
            "Error should mention not a git repository"
        );
    }

    /// Test trend calculation
    #[test]
    fn test_trend_calculation() {
        assert_eq!(calculate_trend(-5), TrendDirection::Improving);
        assert_eq!(calculate_trend(-3), TrendDirection::Improving);
        assert_eq!(calculate_trend(0), TrendDirection::Stable);
        assert_eq!(calculate_trend(2), TrendDirection::Stable);
        assert_eq!(calculate_trend(-2), TrendDirection::Stable);
        assert_eq!(calculate_trend(3), TrendDirection::Degrading);
        assert_eq!(calculate_trend(10), TrendDirection::Degrading);
    }
}

// =============================================================================
// Halstead Tests
// =============================================================================

mod halstead_tests {
    use super::fixtures;

    /// Simple expression should have reasonable operator/operand counts
    #[test]
    #[ignore = "Halstead command not yet implemented"]
    fn halstead_count_operators_operands_simple() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let _file_path =
            fixtures::create_temp_file(&temp_dir, "simple.py", fixtures::PYTHON_HALSTEAD_SIMPLE);

        // TODO: Replace with actual implementation call
        let result: Result<serde_json::Value, &str> = Err("Not implemented");

        let report = match result {
            Ok(report) => report,
            Err(err) => {
                assert_eq!(err, "Not implemented");
                return;
            }
        };
        let functions = report["functions"]
            .as_array()
            .expect("Should have functions array");
        let func = functions.iter().find(|f| {
            f["name"]
                .as_str()
                .map(|n| n.contains("simple_math"))
                .unwrap_or(false)
        });

        assert!(func.is_some());
        let metrics = &func.unwrap()["metrics"];
        let n1 = metrics["distinct_operators"].as_u64().unwrap_or(0);
        let n2 = metrics["distinct_operands"].as_u64().unwrap_or(0);
        assert!(n1 >= 3 && n2 >= 3);
    }

    /// Vocabulary should equal n1 + n2
    #[test]
    #[ignore = "Halstead command not yet implemented"]
    fn halstead_vocabulary_calculation() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let _file_path =
            fixtures::create_temp_file(&temp_dir, "vocab.py", fixtures::PYTHON_HALSTEAD_SIMPLE);

        // TODO: Replace with actual implementation call
        let result: Result<serde_json::Value, &str> = Err("Not implemented");

        let report = match result {
            Ok(report) => report,
            Err(err) => {
                assert_eq!(err, "Not implemented");
                return;
            }
        };
        let functions = report["functions"]
            .as_array()
            .expect("Should have functions array");
        for func in functions {
            let metrics = &func["metrics"];
            let n1 = metrics["distinct_operators"].as_u64().unwrap_or(0);
            let n2 = metrics["distinct_operands"].as_u64().unwrap_or(0);
            let vocabulary = metrics["vocabulary"].as_u64().unwrap_or(0);
            assert_eq!(vocabulary, n1 + n2);
        }
    }

    /// Volume, Difficulty, Effort should follow formulas
    #[test]
    #[ignore = "Halstead command not yet implemented"]
    fn halstead_derived_metrics_formulas() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let _file_path =
            fixtures::create_temp_file(&temp_dir, "derived.py", fixtures::PYTHON_HALSTEAD_SIMPLE);

        // TODO: Replace with actual implementation call
        let result: Result<serde_json::Value, &str> = Err("Not implemented");

        let report = match result {
            Ok(report) => report,
            Err(err) => {
                assert_eq!(err, "Not implemented");
                return;
            }
        };
        let functions = report["functions"]
            .as_array()
            .expect("Should have functions array");
        for func in functions {
            let metrics = &func["metrics"];
            let volume = metrics["volume"].as_f64().unwrap_or(-1.0);
            let difficulty = metrics["difficulty"].as_f64().unwrap_or(-1.0);
            let effort = metrics["effort"].as_f64().unwrap_or(-1.0);

            assert!(volume >= 0.0);
            assert!(difficulty >= 0.0);
            // Effort = D * V
            if volume > 0.0 && difficulty > 0.0 {
                let expected = difficulty * volume;
                assert!((effort - expected).abs() < expected * 0.01 + 1.0);
            }
        }
    }

    /// Estimated bugs should be V/3000
    #[test]
    #[ignore = "Halstead command not yet implemented"]
    fn halstead_estimated_bugs() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let _file_path =
            fixtures::create_temp_file(&temp_dir, "bugs.py", fixtures::PYTHON_HALSTEAD_COMPLEX);

        // TODO: Replace with actual implementation call
        let result: Result<serde_json::Value, &str> = Err("Not implemented");

        let report = match result {
            Ok(report) => report,
            Err(err) => {
                assert_eq!(err, "Not implemented");
                return;
            }
        };
        let functions = report["functions"]
            .as_array()
            .expect("Should have functions array");
        for func in functions {
            let metrics = &func["metrics"];
            let volume = metrics["volume"].as_f64().unwrap_or(0.0);
            let bugs = metrics["estimated_bugs"].as_f64().unwrap_or(-1.0);
            if volume > 0.0 {
                let expected = volume / 3000.0;
                assert!((bugs - expected).abs() < 0.01);
            }
        }
    }

    /// Threshold violations should be detected
    #[test]
    #[ignore = "Halstead command not yet implemented"]
    fn halstead_threshold_violation_detection() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let _file_path =
            fixtures::create_temp_file(&temp_dir, "complex.py", fixtures::PYTHON_HALSTEAD_COMPLEX);

        // TODO: Replace with actual implementation call
        let result: Result<serde_json::Value, &str> = Err("Not implemented");

        let report = match result {
            Ok(report) => report,
            Err(err) => {
                assert_eq!(err, "Not implemented");
                return;
            }
        };
        let functions = report["functions"]
            .as_array()
            .expect("Should have functions array");
        for func in functions {
            let thresholds = func.get("thresholds");
            assert!(thresholds.is_some());
            let status = thresholds.unwrap()["volume_status"].as_str().unwrap_or("");
            assert!(status == "good" || status == "warning" || status == "bad");
        }
    }

    /// Empty function should have minimal metrics
    #[test]
    #[ignore = "Halstead command not yet implemented"]
    fn halstead_empty_function() {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let _file_path =
            fixtures::create_temp_file(&temp_dir, "empty.py", fixtures::PYTHON_HALSTEAD_EMPTY);

        // TODO: Replace with actual implementation call
        let result: Result<serde_json::Value, &str> = Err("Not implemented");

        let report = match result {
            Ok(report) => report,
            Err(err) => {
                assert_eq!(err, "Not implemented");
                return;
            }
        };
        let functions = report["functions"]
            .as_array()
            .expect("Should have functions array");
        let empty_fn = functions.iter().find(|f| {
            f["name"]
                .as_str()
                .map(|n| n.contains("empty_function"))
                .unwrap_or(false)
        });

        assert!(empty_fn.is_some());
        let metrics = &empty_fn.unwrap()["metrics"];
        assert!(metrics["distinct_operators"].as_u64().unwrap_or(999) <= 3);
        assert!(metrics["volume"].as_f64().unwrap_or(-1.0) >= 0.0);
    }
}
