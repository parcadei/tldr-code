//! Integration tests for Quality and Security modules (Phase 8)

use std::path::PathBuf;

use tldr_core::quality::maintainability::maintainability_index;
use tldr_core::quality::smells::{detect_smells, SmellType, ThresholdPreset};
use tldr_core::security::secrets::{scan_secrets, Severity};
use tldr_core::security::vuln::{scan_vulnerabilities, VulnType};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures")
}

// =============================================================================
// smells tests
// =============================================================================

mod smells_tests {
    use super::*;

    #[test]
    fn smells_detects_god_class() {
        let project = fixtures_dir().join("quality/god_class.py");
        if !project.exists() {
            eprintln!("Skipping test - fixture not found: {:?}", project);
            return;
        }

        let result = detect_smells(
            &project,
            ThresholdPreset::Default,
            Some(SmellType::GodClass),
            false,
        );
        assert!(result.is_ok(), "Should succeed: {:?}", result);

        let report = result.unwrap();
        assert!(
            report
                .smells
                .iter()
                .any(|s| matches!(s.smell_type, SmellType::GodClass)),
            "Should detect god class smell"
        );
    }

    #[test]
    fn smells_detects_long_parameter_list() {
        let project = fixtures_dir().join("quality/long_params.py");
        if !project.exists() {
            eprintln!("Skipping test - fixture not found: {:?}", project);
            return;
        }

        let result = detect_smells(
            &project,
            ThresholdPreset::Default,
            Some(SmellType::LongParameterList),
            false,
        );
        assert!(result.is_ok(), "Should succeed: {:?}", result);

        let report = result.unwrap();
        assert!(
            report
                .smells
                .iter()
                .any(|s| matches!(s.smell_type, SmellType::LongParameterList)),
            "Should detect long parameter list smell"
        );
    }

    #[test]
    fn smells_provides_suggestions_when_requested() {
        let project = fixtures_dir().join("quality/god_class.py");
        if !project.exists() {
            return;
        }

        let result = detect_smells(&project, ThresholdPreset::Default, None, true);
        assert!(result.is_ok());

        let report = result.unwrap();
        for smell in &report.smells {
            assert!(
                smell.suggestion.is_some(),
                "Should have suggestion when suggest=true"
            );
        }
    }

    #[test]
    fn smells_strict_finds_more_than_relaxed() {
        let project = fixtures_dir().join("quality");
        if !project.exists() {
            return;
        }

        let strict = detect_smells(&project, ThresholdPreset::Strict, None, false);
        let relaxed = detect_smells(&project, ThresholdPreset::Relaxed, None, false);

        if let (Ok(strict), Ok(relaxed)) = (strict, relaxed) {
            assert!(
                strict.smells.len() >= relaxed.smells.len(),
                "Strict should find at least as many smells as relaxed"
            );
        }
    }
}

// =============================================================================
// maintainability tests
// =============================================================================

mod maintainability_tests {
    use super::*;

    #[test]
    fn maintainability_calculates_mi_score() {
        let project = fixtures_dir().join("quality/grade_a.py");
        if !project.exists() {
            eprintln!("Skipping test - fixture not found: {:?}", project);
            return;
        }

        let result = maintainability_index(&project, false, None);
        assert!(result.is_ok(), "Should succeed: {:?}", result);

        let report = result.unwrap();
        assert!(!report.files.is_empty(), "Should have analyzed files");

        let file_mi = &report.files[0];
        assert!(
            file_mi.mi >= 0.0 && file_mi.mi <= 100.0,
            "MI should be 0-100"
        );
    }

    #[test]
    fn maintainability_assigns_correct_grade() {
        let project = fixtures_dir().join("quality/grade_a.py");
        if !project.exists() {
            return;
        }

        let result = maintainability_index(&project, false, None);
        assert!(result.is_ok());

        let report = result.unwrap();
        // Simple file should get a good grade (A or B)
        let grade = report.files[0].grade;
        assert!(
            grade == 'A' || grade == 'B',
            "Simple file should get grade A or B, got {}",
            grade
        );
    }

    #[test]
    fn maintainability_includes_halstead_when_requested() {
        let project = fixtures_dir().join("quality/grade_a.py");
        if !project.exists() {
            return;
        }

        let result = maintainability_index(&project, true, None);
        assert!(result.is_ok());

        let report = result.unwrap();
        assert!(
            report.files[0].halstead.is_some(),
            "Should include Halstead metrics when requested"
        );

        let halstead = report.files[0].halstead.as_ref().unwrap();
        assert!(halstead.volume > 0.0, "Halstead volume should be positive");
    }

    #[test]
    fn maintainability_summarizes_directory() {
        let project = fixtures_dir().join("quality");
        if !project.exists() {
            return;
        }

        let result = maintainability_index(&project, false, None);
        assert!(result.is_ok());

        let report = result.unwrap();
        assert!(report.summary.files_analyzed > 0, "Should analyze files");
        assert!(report.summary.average_mi > 0.0, "Should have average MI");
    }
}

// =============================================================================
// secrets tests
// =============================================================================

mod secrets_tests {
    use super::*;

    #[test]
    fn secrets_detects_aws_access_key() {
        let project = fixtures_dir().join("security/aws_key.py");
        if !project.exists() {
            eprintln!("Skipping test - fixture not found: {:?}", project);
            return;
        }

        let result = scan_secrets(&project, 4.5, false, None);
        assert!(result.is_ok(), "Should succeed: {:?}", result);

        let report = result.unwrap();
        assert!(
            report.findings.iter().any(|f| f.pattern.contains("AWS")),
            "Should detect AWS key pattern"
        );
    }

    #[test]
    fn secrets_detects_private_key_header() {
        let project = fixtures_dir().join("security/private_key.py");
        if !project.exists() {
            return;
        }

        let result = scan_secrets(&project, 4.5, false, None);
        assert!(result.is_ok());

        let report = result.unwrap();
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.pattern.contains("Private Key")),
            "Should detect private key pattern"
        );
    }

    #[test]
    fn secrets_masks_sensitive_values() {
        let project = fixtures_dir().join("security/aws_key.py");
        if !project.exists() {
            return;
        }

        let result = scan_secrets(&project, 4.5, false, None);
        assert!(result.is_ok());

        let report = result.unwrap();
        for finding in &report.findings {
            // Masked value should contain asterisks or be short
            assert!(
                finding.masked_value.contains('*') || finding.masked_value.len() < 20,
                "Value should be masked: {}",
                finding.masked_value
            );
        }
    }

    #[test]
    fn secrets_assigns_severity_levels() {
        let project = fixtures_dir().join("security");
        if !project.exists() {
            return;
        }

        let result = scan_secrets(&project, 4.5, true, None);
        assert!(result.is_ok());

        let report = result.unwrap();
        // AWS keys should be Critical
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.severity == Severity::Critical),
            "Should have critical severity findings"
        );
    }

    #[test]
    fn secrets_filters_by_severity() {
        let project = fixtures_dir().join("security");
        if !project.exists() {
            return;
        }

        let result = scan_secrets(&project, 4.5, true, Some(Severity::Critical));
        assert!(result.is_ok());

        let report = result.unwrap();
        assert!(
            report
                .findings
                .iter()
                .all(|f| f.severity >= Severity::Critical),
            "Should only have critical or higher findings"
        );
    }

    #[test]
    fn secrets_reports_file_and_line() {
        let project = fixtures_dir().join("security/aws_key.py");
        if !project.exists() {
            return;
        }

        let result = scan_secrets(&project, 4.5, false, None);
        assert!(result.is_ok());

        let report = result.unwrap();
        for finding in &report.findings {
            assert!(!finding.file.as_os_str().is_empty(), "Should have file");
            assert!(finding.line > 0, "Should have line number");
        }
    }
}

// =============================================================================
// vuln tests
// =============================================================================

mod vuln_tests {
    use super::*;

    #[test]
    fn vuln_detects_sql_injection() {
        let project = fixtures_dir().join("security/sql_injection.py");
        if !project.exists() {
            eprintln!("Skipping test - fixture not found: {:?}", project);
            return;
        }

        let result = scan_vulnerabilities(&project, None, Some(VulnType::SqlInjection));
        assert!(result.is_ok(), "Should succeed: {:?}", result);

        let report = result.unwrap();
        // Should detect the SQL injection from request.args -> cursor.execute
        assert!(
            report
                .findings
                .iter()
                .any(|f| matches!(f.vuln_type, VulnType::SqlInjection)),
            "Should detect SQL injection vulnerability"
        );
    }

    #[test]
    fn vuln_detects_command_injection() {
        let project = fixtures_dir().join("security/command_injection.py");
        if !project.exists() {
            return;
        }

        let result = scan_vulnerabilities(&project, None, Some(VulnType::CommandInjection));
        assert!(result.is_ok());

        let report = result.unwrap();
        assert!(
            report
                .findings
                .iter()
                .any(|f| matches!(f.vuln_type, VulnType::CommandInjection)),
            "Should detect command injection vulnerability"
        );
    }

    #[test]
    fn vuln_identifies_sources_and_sinks() {
        let project = fixtures_dir().join("security/sql_injection.py");
        if !project.exists() {
            return;
        }

        let result = scan_vulnerabilities(&project, None, None);
        assert!(result.is_ok());

        let report = result.unwrap();
        for finding in &report.findings {
            assert!(
                !finding.source.variable.is_empty(),
                "Should have source variable"
            );
            assert!(
                !finding.sink.function.is_empty(),
                "Should have sink function"
            );
        }
    }

    #[test]
    fn vuln_provides_remediation() {
        let project = fixtures_dir().join("security/sql_injection.py");
        if !project.exists() {
            return;
        }

        let result = scan_vulnerabilities(&project, None, None);
        assert!(result.is_ok());

        let report = result.unwrap();
        for finding in &report.findings {
            assert!(
                !finding.remediation.is_empty(),
                "Should have remediation advice"
            );
            assert!(finding.cwe_id.is_some(), "Should have CWE ID");
        }
    }

    #[test]
    fn vuln_scans_directory() {
        let project = fixtures_dir().join("security");
        if !project.exists() {
            return;
        }

        let result = scan_vulnerabilities(&project, None, None);
        assert!(result.is_ok());

        let report = result.unwrap();
        assert!(report.files_scanned > 0, "Should scan multiple files");
    }
}
