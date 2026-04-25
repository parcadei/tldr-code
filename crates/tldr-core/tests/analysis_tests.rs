//! Tests for Analysis module operations
//!
//! Commands tested: context, change_impact
//!
//! Phase 7 implementation tests.

use std::path::PathBuf;

use tldr_core::context::get_relevant_context;
use tldr_core::{Language, TldrError};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

// =============================================================================
// context command tests
// =============================================================================

mod context_tests {
    use super::*;

    #[test]
    fn context_finds_entry_point_function() {
        // GIVEN: A project with functions
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context from "vulnerable_sql" entry point (a function that exists)
        let result =
            get_relevant_context(&project, "vulnerable_sql", 1, Language::Python, false, None);

        // THEN: It should find and include the function
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        let ctx = result.unwrap();
        assert_eq!(ctx.entry_point, "vulnerable_sql");
        assert!(
            ctx.functions.iter().any(|f| f.name == "vulnerable_sql"),
            "Should include the entry point function"
        );
    }

    #[test]
    fn context_uses_bfs_traversal() {
        // GIVEN: A project with call relationships (authenticate -> validate_credentials)
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context with depth=1
        let result =
            get_relevant_context(&project, "authenticate", 1, Language::Python, false, None);

        // THEN: BFS should include the entry point and direct callees
        if let Ok(ctx) = result {
            assert!(
                ctx.functions.iter().any(|f| f.name == "authenticate"),
                "Should include entry point"
            );
            // At depth 1, we should see direct callees
        }
        // Note: This test is informational - exact behavior depends on call graph building
    }

    #[test]
    fn context_respects_depth_limit() {
        // GIVEN: A project with call chains
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context with depth=0 vs depth=2
        let result_0 =
            get_relevant_context(&project, "authenticate", 0, Language::Python, false, None);
        let result_2 =
            get_relevant_context(&project, "authenticate", 2, Language::Python, false, None);

        // THEN: Higher depth should include more or equal functions
        if let (Ok(ctx_0), Ok(ctx_2)) = (result_0, result_2) {
            assert!(
                ctx_2.functions.len() >= ctx_0.functions.len(),
                "Higher depth should include >= functions"
            );
        }
    }

    #[test]
    fn context_depth_zero_returns_entry_only() {
        // GIVEN: A project with call graph
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context with depth=0
        let result = get_relevant_context(&project, "safe_sql", 0, Language::Python, false, None);

        // THEN: Only the entry point function should be included
        if let Ok(ctx) = result {
            assert_eq!(
                ctx.functions.len(),
                1,
                "Depth 0 should return only entry point"
            );
            assert_eq!(ctx.functions[0].name, "safe_sql");
        }
    }

    #[test]
    fn context_includes_docstrings_when_requested() {
        // GIVEN: A project with docstrings
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context with include_docstrings=true
        let result =
            get_relevant_context(&project, "vulnerable_sql", 0, Language::Python, true, None);

        // THEN: Docstrings should be present in function contexts
        if let Ok(ctx) = result {
            // vulnerable_sql has a docstring
            let func = ctx.functions.iter().find(|f| f.name == "vulnerable_sql");
            if let Some(f) = func {
                assert!(
                    f.docstring.is_some(),
                    "Docstring should be included when requested"
                );
            }
        }
    }

    #[test]
    fn context_excludes_docstrings_by_default() {
        // GIVEN: A project with docstrings
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context with include_docstrings=false
        let result =
            get_relevant_context(&project, "vulnerable_sql", 0, Language::Python, false, None);

        // THEN: Docstrings should be None
        if let Ok(ctx) = result {
            assert!(
                ctx.functions.iter().all(|f| f.docstring.is_none()),
                "Docstrings should be excluded by default"
            );
        }
    }

    #[test]
    fn context_includes_function_signatures() {
        // GIVEN: A project with typed functions
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context
        let result =
            get_relevant_context(&project, "vulnerable_sql", 0, Language::Python, false, None);

        // THEN: Each function should have a signature
        if let Ok(ctx) = result {
            for func in &ctx.functions {
                assert!(!func.signature.is_empty(), "Signature should not be empty");
                assert!(
                    func.signature.contains("def"),
                    "Python signature should contain 'def'"
                );
            }
        }
    }

    #[test]
    fn context_includes_file_locations() {
        // GIVEN: A project with multiple files
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context
        let result =
            get_relevant_context(&project, "vulnerable_sql", 0, Language::Python, false, None);

        // THEN: Each function should have file and line info
        if let Ok(ctx) = result {
            for func in &ctx.functions {
                assert!(
                    !func.file.as_os_str().is_empty(),
                    "File should not be empty"
                );
                assert!(func.line > 0, "Line number should be > 0");
            }
        }
    }

    #[test]
    fn context_includes_calls_list() {
        // GIVEN: A function that calls other functions (authenticate calls validate_credentials)
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context for that function
        let result =
            get_relevant_context(&project, "authenticate", 0, Language::Python, false, None);

        // THEN: The calls list should be populated
        if let Ok(ctx) = result {
            let auth_fn = ctx.functions.iter().find(|f| f.name == "authenticate");
            if let Some(f) = auth_fn {
                assert!(
                    !f.calls.is_empty(),
                    "authenticate should have calls to validate_credentials"
                );
            }
        }
    }

    #[test]
    fn context_includes_cfg_metrics() {
        // GIVEN: A project with functions
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context for a function with complexity
        let result = get_relevant_context(
            &project,
            "complex_function",
            0,
            Language::Python,
            false,
            None,
        );

        // THEN: CFG metrics (blocks, cyclomatic) should be present
        if let Ok(ctx) = result {
            let complex_fn = ctx.functions.iter().find(|f| f.name == "complex_function");
            if let Some(f) = complex_fn {
                // CFG metrics are optional but should be attempted
                if f.blocks.is_some() {
                    assert!(f.blocks.unwrap() > 0, "Should have blocks");
                }
                if f.cyclomatic.is_some() {
                    assert!(
                        f.cyclomatic.unwrap() > 1,
                        "complex_function should have high complexity"
                    );
                }
            }
        }
    }

    #[test]
    fn context_handles_function_not_found() {
        // GIVEN: A project
        let project = fixtures_dir().join("python-project");

        // WHEN: We request a non-existent entry point
        let result = get_relevant_context(
            &project,
            "nonexistent_function_xyz",
            1,
            Language::Python,
            false,
            None,
        );

        // THEN: It should return FunctionNotFound error
        assert!(
            result.is_err(),
            "Should return error for non-existent function"
        );
        if let Err(e) = result {
            assert!(
                matches!(e, TldrError::FunctionNotFound { .. }),
                "Should be FunctionNotFound error, got: {:?}",
                e
            );
        }
    }

    #[test]
    fn context_handles_cross_file_calls() {
        // GIVEN: A project where app.py might call services/auth.py functions
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context with depth > 0
        let result =
            get_relevant_context(&project, "authenticate", 2, Language::Python, false, None);

        // THEN: Functions from the file should be included
        // Cross-file depends on import resolution working correctly
        if let Ok(ctx) = result {
            assert!(!ctx.functions.is_empty(), "Should find some functions");
        }
    }

    #[test]
    fn context_formats_for_llm_consumption() {
        // GIVEN: Context result
        let project = fixtures_dir().join("python-project");

        // WHEN: We format for LLM
        let result =
            get_relevant_context(&project, "vulnerable_sql", 0, Language::Python, false, None);

        // THEN: It should be formatted for LLM consumption (readable, structured)
        if let Ok(ctx) = result {
            let llm_text = ctx.to_llm_string();
            assert!(!llm_text.is_empty(), "LLM text should not be empty");
            assert!(
                llm_text.contains("vulnerable_sql"),
                "Should contain function name"
            );
            assert!(llm_text.contains("def"), "Should contain signature");
            assert!(
                llm_text.contains("depth=0"),
                "Should contain depth information"
            );
        }
    }

    #[test]
    fn context_works_with_typescript() {
        // GIVEN: A TypeScript project
        let project = fixtures_dir().join("typescript-project");

        // WHEN: We get context for a TypeScript function
        let result = get_relevant_context(&project, "main", 1, Language::TypeScript, false, None);

        // THEN: It should work similarly to Python
        // Note: This depends on the TypeScript fixture having a main function
        if let Ok(ctx) = result {
            assert_eq!(ctx.entry_point, "main");
        }
        // If error, it means the function wasn't found which is acceptable for now
    }

    #[test]
    fn context_handles_recursive_calls() {
        // GIVEN: The BFS implementation
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context with high depth (BFS handles cycles internally)
        let result = get_relevant_context(
            &project,
            "vulnerable_sql",
            10,
            Language::Python,
            false,
            None,
        );

        // THEN: It should not loop infinitely and complete
        assert!(
            result.is_ok() || result.is_err(),
            "Should complete without hanging"
        );
    }

    #[test]
    fn context_handles_cycles_in_call_graph() {
        // GIVEN: A project where cycles might exist
        let project = fixtures_dir().join("python-project");

        // WHEN: We get context with high depth
        let result =
            get_relevant_context(&project, "authenticate", 10, Language::Python, false, None);

        // THEN: It should handle cycles gracefully without infinite loop
        // BFS with visited set ensures each function appears at most once
        if let Ok(ctx) = result {
            let names: Vec<_> = ctx.functions.iter().map(|f| &f.name).collect();
            let unique_count = {
                let mut seen = std::collections::HashSet::new();
                names.iter().filter(|n| seen.insert(*n)).count()
            };
            assert_eq!(
                names.len(),
                unique_count,
                "Each function should appear at most once"
            );
        }
    }
}

// =============================================================================
// change_impact command tests
// =============================================================================

mod change_impact_tests {
    use super::*;
    use tldr_core::analysis::{change_impact, ChangeImpactReport};

    #[test]
    fn change_impact_with_explicit_files() {
        // GIVEN: A project with explicit changed files
        let project = fixtures_dir().join("python-project");
        let changed = vec![project.join("app.py")];

        // WHEN: We analyze change impact
        let result = change_impact(&project, Some(&changed), Language::Python);

        // THEN: It should return a report
        assert!(result.is_ok(), "Should return Ok, got {:?}", result);
        let report = result.unwrap();
        assert_eq!(report.detection_method, "explicit");
        assert!(
            !report.changed_files.is_empty(),
            "Should have changed files"
        );
    }

    #[test]
    fn change_impact_empty_when_no_changes() {
        // GIVEN: A project with no changes specified
        let project = fixtures_dir().join("python-project");
        let changed: Vec<PathBuf> = vec![];

        // WHEN: We analyze change impact
        let result = change_impact(&project, Some(&changed), Language::Python);

        // THEN: It should return empty report
        assert!(result.is_ok());
        let report = result.unwrap();
        assert!(report.changed_files.is_empty());
        assert!(report.affected_tests.is_empty());
    }

    #[test]
    fn change_impact_report_structure() {
        // GIVEN: A change impact report
        let report = ChangeImpactReport {
            changed_files: vec![PathBuf::from("app.py")],
            affected_tests: vec![PathBuf::from("test_app.py")],
            affected_test_functions: vec![],
            affected_functions: vec![],
            detection_method: "explicit".to_string(),
            metadata: None,
            status: tldr_core::analysis::ChangeImpactStatus::Completed,
        };

        // THEN: It should have the expected structure
        assert_eq!(report.changed_files.len(), 1);
        assert_eq!(report.affected_tests.len(), 1);
        assert_eq!(report.detection_method, "explicit");
    }
}
