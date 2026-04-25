//! L1 commodity tool types and conversion impls
//!
//! Defines types for the L1 diagnostic tool orchestration layer:
//! - `ToolCategory`: classification of tools (linter, security scanner, etc.)
//! - `ToolConfig`: static configuration for a single diagnostic tool
//! - `ToolResult`: execution result from running a tool
//! - `L1Finding`: raw finding from a tool before conversion to `BugbotFinding`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::types::BugbotFinding;

/// Category of commodity diagnostic tool
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    /// Language type checker (e.g., pyright, tsc). Not used for Rust
    /// since clippy subsumes cargo check.
    TypeChecker,
    /// Linter (e.g., clippy, eslint)
    Linter,
    /// Security vulnerability scanner (e.g., cargo-audit)
    SecurityScanner,
}

/// Configuration for a single diagnostic tool
///
/// Uses `&'static str` and `&'static [&'static str]` for zero-allocation registry.
#[derive(Debug, Clone)]
pub struct ToolConfig {
    /// Display name (e.g., "clippy", "cargo-audit")
    pub name: &'static str,
    /// Binary to execute (e.g., "cargo")
    pub binary: &'static str,
    /// Binary to check for availability (e.g., "cargo-clippy").
    /// Different from `binary` for cargo subcommands where the main
    /// binary is "cargo" but detection needs "cargo-clippy". [PM-2]
    pub detection_binary: &'static str,
    /// Arguments to pass (e.g., ["clippy", "--message-format=json"])
    pub args: &'static [&'static str],
    /// Tool category
    pub category: ToolCategory,
    /// Parser identifier (e.g., "cargo", "cargo-audit")
    pub parser: &'static str,
}

/// Result from running a single tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Tool name
    pub name: String,
    /// Tool category
    pub category: ToolCategory,
    /// Whether the tool ran successfully
    pub success: bool,
    /// Execution time in milliseconds
    pub duration_ms: u64,
    /// Number of findings produced
    pub finding_count: usize,
    /// Error message if the tool failed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Process exit code
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// L1 finding from a commodity tool before conversion to `BugbotFinding`.
///
/// The `tool` field is set by `ToolRunner` after parsing, not by the parser
/// itself. Parsers set `tool` to an empty string. [PM-6]
#[derive(Debug, Clone)]
pub struct L1Finding {
    /// Tool that produced the finding (set by runner, not parser)
    pub tool: String,
    /// Tool category
    pub category: ToolCategory,
    /// File path (relative to project root)
    pub file: PathBuf,
    /// Line number
    pub line: u32,
    /// Column number
    pub column: u32,
    /// Severity as reported by the tool (e.g., "warning", "error")
    pub native_severity: String,
    /// Normalized severity: "high", "medium", "low", "info"
    pub severity: String,
    /// Human-readable description
    pub message: String,
    /// Tool-specific error/lint code (e.g., "clippy::needless_return")
    pub code: Option<String>,
}

impl From<L1Finding> for BugbotFinding {
    fn from(l1: L1Finding) -> Self {
        BugbotFinding {
            finding_type: format!("tool:{}", l1.tool),
            severity: l1.severity,
            file: l1.file,
            function: String::new(), // L1 findings lack function context
            line: l1.line as usize,
            message: l1.message,
            evidence: serde_json::json!({
                "tool": l1.tool,
                "category": format!("{:?}", l1.category),
                "code": l1.code,
                "native_severity": l1.native_severity,
                "column": l1.column,
            }),
            confidence: None,
            finding_id: None,
        }
    }
}

/// Registry of commodity diagnostic tools per language.
///
/// Maps language names (lowercase strings like "rust", "python") to their
/// configured diagnostic tools. The default registry includes:
/// - Rust: clippy + cargo-audit (NO cargo check -- clippy subsumes it) [PM-1]
///
/// Uses `detection_binary` (not `binary`) for availability probing [PM-2].
pub struct ToolRegistry {
    registry: HashMap<String, Vec<ToolConfig>>,
}

impl ToolRegistry {
    /// Create a new registry with default tool registrations.
    ///
    /// Default Rust tools:
    /// - clippy (linter, detection_binary: "cargo-clippy")
    /// - cargo-audit (security scanner, detection_binary: "cargo-audit")
    ///
    /// CRITICAL [PM-1]: cargo check is NOT included. Clippy subsumes it and
    /// running both would produce duplicate diagnostics plus double compile time.
    pub fn new() -> Self {
        let mut registry = HashMap::new();

        // Rust tools -- ONLY clippy + cargo-audit [PM-1]: cargo check removed,
        // clippy subsumes it.
        registry.insert(
            "rust".to_string(),
            vec![
                ToolConfig {
                    name: "clippy",
                    binary: "cargo",
                    detection_binary: "cargo-clippy", // [PM-2]
                    args: &["clippy", "--message-format=json", "--", "-W", "clippy::all"],
                    category: ToolCategory::Linter,
                    parser: "cargo",
                },
                ToolConfig {
                    name: "cargo-audit",
                    binary: "cargo",
                    detection_binary: "cargo-audit", // [PM-2]
                    args: &["audit", "--json"],
                    category: ToolCategory::SecurityScanner,
                    parser: "cargo-audit",
                },
            ],
        );

        // Python tools -- ruff (fast linter) + pyright (type checker)
        registry.insert(
            "python".to_string(),
            vec![
                ToolConfig {
                    name: "ruff",
                    binary: "ruff",
                    detection_binary: "ruff",
                    args: &["check", "--select=E,F,B,S", "--output-format=json", "."],
                    category: ToolCategory::Linter,
                    parser: "ruff",
                },
                ToolConfig {
                    name: "pyright",
                    binary: "pyright",
                    detection_binary: "pyright",
                    args: &["--outputjson", "."],
                    category: ToolCategory::TypeChecker,
                    parser: "pyright",
                },
            ],
        );

        // JavaScript tools -- eslint
        registry.insert(
            "javascript".to_string(),
            vec![ToolConfig {
                name: "eslint",
                binary: "eslint",
                detection_binary: "eslint",
                args: &["--format", "json", "."],
                category: ToolCategory::Linter,
                parser: "eslint",
            }],
        );

        // TypeScript tools -- eslint (tsc is too slow for L1)
        registry.insert(
            "typescript".to_string(),
            vec![ToolConfig {
                name: "eslint",
                binary: "eslint",
                detection_binary: "eslint",
                args: &["--format", "json", "."],
                category: ToolCategory::Linter,
                parser: "eslint",
            }],
        );

        // Go tools -- golangci-lint
        registry.insert(
            "go".to_string(),
            vec![ToolConfig {
                name: "golangci-lint",
                binary: "golangci-lint",
                detection_binary: "golangci-lint",
                args: &["run", "--out-format", "json"],
                category: ToolCategory::Linter,
                parser: "golangci-lint",
            }],
        );

        // Ruby tools -- rubocop
        registry.insert(
            "ruby".to_string(),
            vec![ToolConfig {
                name: "rubocop",
                binary: "rubocop",
                detection_binary: "rubocop",
                args: &["--format", "json"],
                category: ToolCategory::Linter,
                parser: "rubocop",
            }],
        );

        // Java tools -- checkstyle (plain format, parsed line by line)
        registry.insert(
            "java".to_string(),
            vec![ToolConfig {
                name: "checkstyle",
                binary: "checkstyle",
                detection_binary: "checkstyle",
                args: &["-c", "/google_checks.xml", "-f", "plain", "."],
                category: ToolCategory::Linter,
                parser: "checkstyle",
            }],
        );

        // Kotlin tools -- ktlint
        registry.insert(
            "kotlin".to_string(),
            vec![ToolConfig {
                name: "ktlint",
                binary: "ktlint",
                detection_binary: "ktlint",
                args: &["--reporter=json"],
                category: ToolCategory::Linter,
                parser: "ktlint",
            }],
        );

        // Swift tools -- swiftlint
        registry.insert(
            "swift".to_string(),
            vec![ToolConfig {
                name: "swiftlint",
                binary: "swiftlint",
                detection_binary: "swiftlint",
                args: &["lint", "--reporter", "json"],
                category: ToolCategory::Linter,
                parser: "swiftlint",
            }],
        );

        // C tools -- cppcheck (tab-separated template output)
        registry.insert(
            "c".to_string(),
            vec![ToolConfig {
                name: "cppcheck",
                binary: "cppcheck",
                detection_binary: "cppcheck",
                args: &[
                    "--enable=all",
                    "--template={file}\t{line}\t{column}\t{severity}\t{id}\t{message}",
                    ".",
                ],
                category: ToolCategory::Linter,
                parser: "cppcheck",
            }],
        );

        // C++ tools -- cppcheck (same parser as C)
        registry.insert(
            "cpp".to_string(),
            vec![ToolConfig {
                name: "cppcheck",
                binary: "cppcheck",
                detection_binary: "cppcheck",
                args: &[
                    "--enable=all",
                    "--language=c++",
                    "--template={file}\t{line}\t{column}\t{severity}\t{id}\t{message}",
                    ".",
                ],
                category: ToolCategory::Linter,
                parser: "cppcheck",
            }],
        );

        // PHP tools -- phpstan
        registry.insert(
            "php".to_string(),
            vec![ToolConfig {
                name: "phpstan",
                binary: "phpstan",
                detection_binary: "phpstan",
                args: &["analyse", "--error-format=json", "--no-progress", "."],
                category: ToolCategory::Linter,
                parser: "phpstan",
            }],
        );

        // Lua tools -- luacheck (plain format, parsed line by line)
        registry.insert(
            "lua".to_string(),
            vec![ToolConfig {
                name: "luacheck",
                binary: "luacheck",
                detection_binary: "luacheck",
                args: &["--formatter", "plain", "."],
                category: ToolCategory::Linter,
                parser: "luacheck",
            }],
        );

        Self { registry }
    }

    /// Get all configured tools for a language.
    ///
    /// Returns an empty `Vec` if the language has no registered tools.
    pub fn tools_for_language(&self, lang: &str) -> Vec<&ToolConfig> {
        self.registry
            .get(lang)
            .map(|tools| tools.iter().collect())
            .unwrap_or_default()
    }

    /// Detect which tools are actually installed on the system.
    ///
    /// Probes `detection_binary` (not `binary`) to check availability [PM-2].
    /// For cargo subcommands, this correctly checks for e.g. "cargo-clippy"
    /// rather than just "cargo".
    ///
    /// Returns `(available, missing)` where each is a list of tool configs.
    pub fn detect_available_tools(&self, lang: &str) -> (Vec<&ToolConfig>, Vec<&ToolConfig>) {
        let all_tools = self.tools_for_language(lang);
        let mut available = Vec::new();
        let mut missing = Vec::new();

        for tool in all_tools {
            if which::which(tool.detection_binary).is_ok() {
                available.push(tool);
            } else {
                missing.push(tool);
            }
        }

        (available, missing)
    }

    /// Register a tool for a language.
    ///
    /// Appends the tool to any existing tools for the language.
    pub fn register_tool(&mut self, lang: &str, config: ToolConfig) {
        self.registry
            .entry(lang.to_string())
            .or_default()
            .push(config);
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_category_serialization() {
        // ToolCategory serializes to snake_case
        let tc = ToolCategory::TypeChecker;
        let json = serde_json::to_string(&tc).unwrap();
        assert_eq!(json, "\"type_checker\"");

        let linter = ToolCategory::Linter;
        let json = serde_json::to_string(&linter).unwrap();
        assert_eq!(json, "\"linter\"");

        let scanner = ToolCategory::SecurityScanner;
        let json = serde_json::to_string(&scanner).unwrap();
        assert_eq!(json, "\"security_scanner\"");

        // Roundtrip
        let deser: ToolCategory = serde_json::from_str("\"security_scanner\"").unwrap();
        assert_eq!(deser, ToolCategory::SecurityScanner);
    }

    #[test]
    fn test_tool_result_serialization() {
        let result = ToolResult {
            name: "clippy".to_string(),
            category: ToolCategory::Linter,
            success: true,
            duration_ms: 1234,
            finding_count: 5,
            error: None,
            exit_code: Some(0),
        };

        let json = serde_json::to_string(&result).unwrap();
        let deser: ToolResult = serde_json::from_str(&json).unwrap();

        assert_eq!(deser.name, "clippy");
        assert_eq!(deser.category, ToolCategory::Linter);
        assert!(deser.success);
        assert_eq!(deser.duration_ms, 1234);
        assert_eq!(deser.finding_count, 5);
        assert!(deser.error.is_none());
        assert_eq!(deser.exit_code, Some(0));
    }

    #[test]
    fn test_l1_finding_to_bugbot_finding() {
        let l1 = L1Finding {
            tool: "clippy".to_string(),
            category: ToolCategory::Linter,
            file: PathBuf::from("src/main.rs"),
            line: 42,
            column: 5,
            native_severity: "warning".to_string(),
            severity: "medium".to_string(),
            message: "unused variable `x`".to_string(),
            code: Some("clippy::unused_variables".to_string()),
        };

        let finding: BugbotFinding = l1.into();

        assert_eq!(finding.finding_type, "tool:clippy");
        assert_eq!(finding.severity, "medium");
        assert_eq!(finding.file, PathBuf::from("src/main.rs"));
        assert!(finding.function.is_empty());
        assert_eq!(finding.line, 42);
        assert_eq!(finding.message, "unused variable `x`");
    }

    #[test]
    fn test_l1_finding_severity_preserved() {
        let l1 = L1Finding {
            tool: "test-tool".to_string(),
            category: ToolCategory::SecurityScanner,
            file: PathBuf::from("Cargo.lock"),
            line: 1,
            column: 1,
            native_severity: "error".to_string(),
            severity: "high".to_string(),
            message: "vulnerability found".to_string(),
            code: Some("RUSTSEC-2024-0001".to_string()),
        };

        let finding: BugbotFinding = l1.into();
        assert_eq!(finding.severity, "high");
    }

    #[test]
    fn test_l1_finding_evidence_contains_tool_info() {
        let l1 = L1Finding {
            tool: "clippy".to_string(),
            category: ToolCategory::Linter,
            file: PathBuf::from("src/lib.rs"),
            line: 10,
            column: 3,
            native_severity: "warning".to_string(),
            severity: "medium".to_string(),
            message: "test".to_string(),
            code: Some("clippy::needless_return".to_string()),
        };

        let finding: BugbotFinding = l1.into();
        let evidence = &finding.evidence;

        assert_eq!(evidence["tool"], "clippy");
        assert_eq!(evidence["category"], "Linter");
        assert_eq!(evidence["code"], "clippy::needless_return");
        assert_eq!(evidence["native_severity"], "warning");
        assert_eq!(evidence["column"], 3);
    }

    #[test]
    fn test_l1_finding_empty_code() {
        let l1 = L1Finding {
            tool: "clippy".to_string(),
            category: ToolCategory::Linter,
            file: PathBuf::from("src/lib.rs"),
            line: 5,
            column: 1,
            native_severity: "error".to_string(),
            severity: "high".to_string(),
            message: "cannot find type".to_string(),
            code: None,
        };

        let finding: BugbotFinding = l1.into();
        assert!(finding.evidence["code"].is_null());
    }

    #[test]
    fn test_tool_result_no_error_skips_field() {
        let result = ToolResult {
            name: "clippy".to_string(),
            category: ToolCategory::Linter,
            success: true,
            duration_ms: 100,
            finding_count: 0,
            error: None,
            exit_code: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(
            !json.contains("\"error\""),
            "error field should be skipped when None, got: {}",
            json
        );
        assert!(
            !json.contains("\"exit_code\""),
            "exit_code field should be skipped when None, got: {}",
            json
        );
    }

    #[test]
    fn test_tool_result_with_error() {
        let result = ToolResult {
            name: "cargo-audit".to_string(),
            category: ToolCategory::SecurityScanner,
            success: false,
            duration_ms: 50,
            finding_count: 0,
            error: Some("binary not found".to_string()),
            exit_code: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(
            json.contains("\"error\""),
            "error field should be present when Some, got: {}",
            json
        );
        assert!(
            json.contains("binary not found"),
            "error message should be serialized, got: {}",
            json
        );
    }

    // =========================================================================
    // ToolRegistry tests
    // =========================================================================

    #[test]
    fn test_registry_rust_tools() {
        // ToolRegistry::new() has exactly 2 Rust tools: clippy + cargo-audit
        let registry = ToolRegistry::new();
        let tools = registry.tools_for_language("rust");
        assert_eq!(tools.len(), 2, "expected 2 Rust tools, got {}", tools.len());

        let names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        assert!(names.contains(&"clippy"), "expected clippy in {:?}", names);
        assert!(
            names.contains(&"cargo-audit"),
            "expected cargo-audit in {:?}",
            names
        );
    }

    #[test]
    fn test_registry_no_cargo_check() {
        // CRITICAL PM-1 regression guard: Rust registry does NOT contain
        // "cargo check" or anything named "check"
        let registry = ToolRegistry::new();
        let tools = registry.tools_for_language("rust");

        for tool in &tools {
            assert_ne!(
                tool.name, "cargo check",
                "PM-1 violation: cargo check must not be in Rust registry"
            );
            assert_ne!(
                tool.name, "check",
                "PM-1 violation: 'check' tool must not be in Rust registry"
            );
        }

        // Also verify no tool has args that would run `cargo check`
        for tool in &tools {
            let args_joined = tool.args.join(" ");
            // clippy args start with "clippy", not "check"
            assert!(
                !args_joined.starts_with("check"),
                "PM-1 violation: tool '{}' has args starting with 'check': {}",
                tool.name,
                args_joined
            );
        }
    }

    #[test]
    fn test_registry_unknown_language() {
        // tools_for_language("unknown") returns empty Vec
        let registry = ToolRegistry::new();
        let tools = registry.tools_for_language("unknown");
        assert!(
            tools.is_empty(),
            "expected empty Vec for unknown language, got {} tools",
            tools.len()
        );
    }

    #[test]
    fn test_registry_clippy_detection_binary() {
        // clippy tool has detection_binary "cargo-clippy", not "cargo" [PM-2]
        let registry = ToolRegistry::new();
        let tools = registry.tools_for_language("rust");
        let clippy = tools.iter().find(|t| t.name == "clippy").unwrap();

        assert_eq!(
            clippy.detection_binary, "cargo-clippy",
            "PM-2: clippy detection_binary should be 'cargo-clippy', got '{}'",
            clippy.detection_binary
        );
        // Verify it's different from binary
        assert_ne!(
            clippy.binary, clippy.detection_binary,
            "PM-2: detection_binary should differ from binary for cargo subcommands"
        );
    }

    #[test]
    fn test_registry_cargo_audit_detection_binary() {
        // cargo-audit tool has detection_binary "cargo-audit", not "cargo" [PM-2]
        let registry = ToolRegistry::new();
        let tools = registry.tools_for_language("rust");
        let audit = tools.iter().find(|t| t.name == "cargo-audit").unwrap();

        assert_eq!(
            audit.detection_binary, "cargo-audit",
            "PM-2: cargo-audit detection_binary should be 'cargo-audit', got '{}'",
            audit.detection_binary
        );
    }

    #[test]
    fn test_registry_clippy_uses_message_format_json() {
        // clippy args include "--message-format=json" for parseable output
        let registry = ToolRegistry::new();
        let tools = registry.tools_for_language("rust");
        let clippy = tools.iter().find(|t| t.name == "clippy").unwrap();

        assert!(
            clippy.args.contains(&"--message-format=json"),
            "clippy args should include '--message-format=json', got {:?}",
            clippy.args
        );
    }

    #[test]
    fn test_registry_cargo_audit_args_include_json() {
        // cargo-audit args include "--json" for JSON output
        let registry = ToolRegistry::new();
        let tools = registry.tools_for_language("rust");
        let audit = tools.iter().find(|t| t.name == "cargo-audit").unwrap();

        assert!(
            audit.args.contains(&"--json"),
            "cargo-audit args should include '--json', got {:?}",
            audit.args
        );
    }

    #[test]
    fn test_detect_available_filters_correctly() {
        // Register a fake tool with a nonexistent detection_binary.
        // It should end up in the missing list.
        let mut registry = ToolRegistry::new();
        registry.register_tool(
            "test-lang",
            ToolConfig {
                name: "fake-tool",
                binary: "nonexistent-binary-xyz-12345",
                detection_binary: "nonexistent-binary-xyz-12345",
                args: &[],
                category: ToolCategory::Linter,
                parser: "cargo",
            },
        );

        let (available, missing) = registry.detect_available_tools("test-lang");

        // The fake tool should be missing
        assert_eq!(
            missing.len(),
            1,
            "expected 1 missing tool, got {}",
            missing.len()
        );
        assert_eq!(missing[0].name, "fake-tool");
        assert!(
            available.is_empty(),
            "expected no available tools for test-lang with fake binary"
        );
    }

    #[test]
    fn test_detect_real_cargo() {
        // cargo should always be available in a Rust dev environment.
        // This test verifies detect_available_tools doesn't panic.
        let registry = ToolRegistry::new();
        let (available, missing) = registry.detect_available_tools("rust");

        // Don't assert specific counts -- CI environments may differ.
        // Just verify the partition covers all tools.
        let tools = registry.tools_for_language("rust");
        assert_eq!(
            available.len() + missing.len(),
            tools.len(),
            "available + missing should equal total tools"
        );
    }

    #[test]
    fn test_detect_unknown_language_returns_empty() {
        let registry = ToolRegistry::new();
        let (available, missing) = registry.detect_available_tools("unknown");
        assert!(available.is_empty());
        assert!(missing.is_empty());
    }

    #[test]
    fn test_register_tool() {
        let mut registry = ToolRegistry::new();

        // Python has ruff by default; registering adds more
        let before = registry.tools_for_language("python").len();

        registry.register_tool(
            "python",
            ToolConfig {
                name: "ruff",
                binary: "ruff",
                detection_binary: "ruff",
                args: &["check", "--output-format=json"],
                category: ToolCategory::Linter,
                parser: "ruff",
            },
        );

        let tools = registry.tools_for_language("python");
        assert_eq!(tools.len(), before + 1);
        assert!(tools.iter().any(|t| t.name == "ruff"));
    }

    #[test]
    fn test_register_tool_appends() {
        // Registering additional tools for a language appends to defaults
        let mut registry = ToolRegistry::new();
        let before = registry.tools_for_language("python").len();

        registry.register_tool(
            "python",
            ToolConfig {
                name: "bandit",
                binary: "bandit",
                detection_binary: "bandit",
                args: &["-f", "json"],
                category: ToolCategory::SecurityScanner,
                parser: "bandit",
            },
        );

        let tools = registry.tools_for_language("python");
        assert_eq!(tools.len(), before + 1);
        assert!(tools.iter().any(|t| t.name == "ruff"));
        assert!(tools.iter().any(|t| t.name == "bandit"));
    }

    #[test]
    fn test_default_impl_matches_new() {
        // Default::default() should produce the same registry as new()
        let from_new = ToolRegistry::new();
        let from_default = ToolRegistry::default();

        let new_tools = from_new.tools_for_language("rust");
        let default_tools = from_default.tools_for_language("rust");

        assert_eq!(new_tools.len(), default_tools.len());
        for (n, d) in new_tools.iter().zip(default_tools.iter()) {
            assert_eq!(n.name, d.name);
            assert_eq!(n.binary, d.binary);
            assert_eq!(n.detection_binary, d.detection_binary);
        }
    }
}
