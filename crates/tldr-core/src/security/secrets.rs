//! Secret scanning
//!
//! Implements detection of hardcoded secrets as per spec Section 2.9.1:
//! - AWS key patterns (AKIA...)
//! - Private key headers (-----BEGIN...PRIVATE KEY-----)
//! - High entropy strings (Shannon entropy > threshold)
//! - Password assignments (password = "...")
//!
//! # Example
//! ```ignore
//! use tldr_core::security::secrets::{scan_secrets, Severity};
//!
//! let report = scan_secrets(Path::new("src/"), 4.5, false, None)?;
//! for finding in &report.findings {
//!     println!("{}: {} at {}:{}", finding.severity, finding.pattern, finding.file.display(), finding.line);
//! }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// Severity levels for secret findings
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Low severity - may be false positive
    Low,
    /// Medium severity - should be reviewed
    Medium,
    /// High severity - likely a real secret
    High,
    /// Critical severity - confirmed sensitive data
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Low => write!(f, "LOW"),
            Severity::Medium => write!(f, "MEDIUM"),
            Severity::High => write!(f, "HIGH"),
            Severity::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// A secret pattern to detect
#[derive(Debug, Clone)]
struct SecretPattern {
    name: &'static str,
    pattern: Regex,
    severity: Severity,
    description: &'static str,
}

/// A single secret finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretFinding {
    /// File containing the secret
    pub file: PathBuf,
    /// Line number
    pub line: u32,
    /// Column number (start of match)
    pub column: u32,
    /// Pattern that matched
    pub pattern: String,
    /// Severity level
    pub severity: Severity,
    /// Masked value (partial redaction)
    pub masked_value: String,
    /// Description of the secret type
    pub description: String,
    /// Full line content (for context)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_content: Option<String>,
}

/// Summary statistics for secret scanning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsSummary {
    /// Total findings
    pub total_findings: usize,
    /// Count by severity
    pub by_severity: HashMap<String, usize>,
    /// Count by pattern type
    pub by_pattern: HashMap<String, usize>,
}

/// Report from secret scanning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsReport {
    /// All secret findings
    pub findings: Vec<SecretFinding>,
    /// Number of files scanned
    pub files_scanned: usize,
    /// Number of patterns checked
    pub patterns_checked: usize,
    /// Summary statistics
    pub summary: SecretsSummary,
}

// =============================================================================
// Secret Patterns
// =============================================================================

lazy_static::lazy_static! {
    /// Compiled secret detection patterns
    static ref SECRET_PATTERNS: Vec<SecretPattern> = vec![
        // AWS Access Key ID
        SecretPattern {
            name: "AWS Access Key",
            pattern: Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
            severity: Severity::Critical,
            description: "AWS Access Key ID detected",
        },
        // AWS Secret Access Key
        SecretPattern {
            name: "AWS Secret Key",
            pattern: Regex::new(r#"(?i)aws(.{0,20})?['"][0-9a-zA-Z/+]{40}['"]"#).unwrap(),
            severity: Severity::Critical,
            description: "AWS Secret Access Key detected",
        },
        // Private Key Header
        SecretPattern {
            name: "Private Key",
            pattern: Regex::new(r"-----BEGIN\s*(RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----").unwrap(),
            severity: Severity::Critical,
            description: "Private key header detected",
        },
        // GitHub Token
        SecretPattern {
            name: "GitHub Token",
            pattern: Regex::new(r"gh[pousr]_[A-Za-z0-9_]{36,}").unwrap(),
            severity: Severity::Critical,
            description: "GitHub personal access token detected",
        },
        // Generic API Key
        SecretPattern {
            name: "API Key",
            pattern: Regex::new(r#"(?i)(api[_-]?key|apikey)\s*[:=]\s*['"]\s*[a-zA-Z0-9]{20,}['"]\s*"#).unwrap(),
            severity: Severity::High,
            description: "Generic API key pattern detected",
        },
        // Password in config
        SecretPattern {
            name: "Password",
            pattern: Regex::new(r#"(?i)(password|passwd|pwd)\s*[:=]\s*['"][^'"]{4,}['"]"#).unwrap(),
            severity: Severity::High,
            description: "Hardcoded password detected",
        },
        // Secret in config
        SecretPattern {
            name: "Secret",
            pattern: Regex::new(r#"(?i)(secret|token)\s*[:=]\s*['"][^'"]{8,}['"]"#).unwrap(),
            severity: Severity::High,
            description: "Hardcoded secret/token detected",
        },
        // Database URL with credentials
        SecretPattern {
            name: "Database URL",
            pattern: Regex::new(r"(?i)(postgres|mysql|mongodb|redis)://[^:]+:[^@]+@").unwrap(),
            severity: Severity::High,
            description: "Database URL with credentials detected",
        },
        // Slack Token
        SecretPattern {
            name: "Slack Token",
            pattern: Regex::new(r"xox[baprs]-[0-9]{10,13}-[0-9]{10,13}[a-zA-Z0-9-]*").unwrap(),
            severity: Severity::Critical,
            description: "Slack token detected",
        },
        // JWT
        SecretPattern {
            name: "JWT",
            pattern: Regex::new(r"eyJ[A-Za-z0-9_-]*\.eyJ[A-Za-z0-9_-]*\.[A-Za-z0-9_-]*").unwrap(),
            severity: Severity::Medium,
            description: "JSON Web Token detected",
        },
        // Bearer Token
        SecretPattern {
            name: "Bearer Token",
            pattern: Regex::new(r#"(?i)bearer\s+[a-zA-Z0-9_\-\.]+[a-zA-Z0-9_\-\.]"#).unwrap(),
            severity: Severity::Medium,
            description: "Bearer token in header detected",
        },
    ];

    /// Test file patterns to skip by default
    static ref TEST_FILE_PATTERNS: Regex = Regex::new(
        r"(?i)(test[_/]|_test\.|\.test\.|spec[_/]|_spec\.|\.spec\.|conftest|fixture|mock)"
    ).unwrap();
}

// =============================================================================
// Main API
// =============================================================================

/// Scan for hardcoded secrets in files
///
/// # Arguments
/// * `path` - File or directory to scan
/// * `entropy_threshold` - Shannon entropy threshold for high-entropy strings (default: 4.5)
/// * `include_test` - Whether to scan test files
/// * `severity_filter` - Optional minimum severity to report
///
/// # Returns
/// * `Ok(SecretsReport)` - Report with all findings
/// * `Err(TldrError)` - On file system errors
///
/// # Example
/// ```ignore
/// use tldr_core::security::secrets::{scan_secrets, Severity};
///
/// // Scan with default settings
/// let report = scan_secrets(Path::new("src/"), 4.5, false, None)?;
///
/// // Scan only for critical findings
/// let report = scan_secrets(Path::new("src/"), 4.5, false, Some(Severity::Critical))?;
/// ```
pub fn scan_secrets(
    path: &Path,
    entropy_threshold: f64,
    include_test: bool,
    severity_filter: Option<Severity>,
) -> TldrResult<SecretsReport> {
    let mut findings = Vec::new();
    let mut files_scanned = 0;

    // Collect files to scan
    let files: Vec<PathBuf> = if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                // Filter by extension (only scan text files)
                let ext = e.path().extension().and_then(|e| e.to_str()).unwrap_or("");
                matches!(
                    ext,
                    "py" | "js"
                        | "ts"
                        | "jsx"
                        | "tsx"
                        | "go"
                        | "rs"
                        | "java"
                        | "rb"
                        | "php"
                        | "yaml"
                        | "yml"
                        | "json"
                        | "toml"
                        | "xml"
                        | "env"
                        | "sh"
                        | "bash"
                        | "zsh"
                        | "config"
                        | "cfg"
                        | "conf"
                        | "properties"
                )
            })
            .filter(|e| {
                // Skip test files unless requested
                include_test || !TEST_FILE_PATTERNS.is_match(&e.path().to_string_lossy())
            })
            .map(|e| e.path().to_path_buf())
            .collect()
    };

    // Scan each file
    for file_path in &files {
        if let Ok(file_findings) = scan_file(file_path, entropy_threshold) {
            findings.extend(file_findings);
            files_scanned += 1;
        }
    }

    // Apply severity filter
    if let Some(min_severity) = severity_filter {
        findings.retain(|f| f.severity >= min_severity);
    }

    // Calculate summary
    let mut by_severity: HashMap<String, usize> = HashMap::new();
    let mut by_pattern: HashMap<String, usize> = HashMap::new();
    for finding in &findings {
        *by_severity.entry(finding.severity.to_string()).or_insert(0) += 1;
        *by_pattern.entry(finding.pattern.clone()).or_insert(0) += 1;
    }

    let summary = SecretsSummary {
        total_findings: findings.len(),
        by_severity,
        by_pattern,
    };

    Ok(SecretsReport {
        findings,
        files_scanned,
        patterns_checked: SECRET_PATTERNS.len(),
        summary,
    })
}

// =============================================================================
// Internal Implementation
// =============================================================================

/// Scan a single file for secrets
fn scan_file(path: &Path, entropy_threshold: f64) -> TldrResult<Vec<SecretFinding>> {
    let content = std::fs::read_to_string(path)?;
    let mut findings = Vec::new();

    for (line_num, line) in content.lines().enumerate() {
        let line_num = (line_num + 1) as u32;

        // Check each pattern
        for pattern in SECRET_PATTERNS.iter() {
            if let Some(mat) = pattern.pattern.find(line) {
                // Skip placeholder/example values for generic patterns
                if is_placeholder_pattern_match(line, pattern.name) {
                    continue;
                }
                findings.push(SecretFinding {
                    file: path.to_path_buf(),
                    line: line_num,
                    column: mat.start() as u32,
                    pattern: pattern.name.to_string(),
                    severity: pattern.severity,
                    masked_value: mask_secret(mat.as_str()),
                    description: pattern.description.to_string(),
                    line_content: Some(truncate_line(line, 100)),
                });
            }
        }

        // Check for high-entropy strings
        for word in extract_strings(line) {
            if word.len() >= 16 && shannon_entropy(&word) > entropy_threshold {
                // Skip if it looks like a common non-secret pattern
                if !is_likely_false_positive(&word) {
                    findings.push(SecretFinding {
                        file: path.to_path_buf(),
                        line: line_num,
                        column: line.find(&word).unwrap_or(0) as u32,
                        pattern: "High Entropy".to_string(),
                        severity: Severity::Medium,
                        masked_value: mask_secret(&word),
                        description: format!(
                            "High entropy string detected (entropy: {:.2})",
                            shannon_entropy(&word)
                        ),
                        line_content: Some(truncate_line(line, 100)),
                    });
                }
            }
        }
    }

    Ok(findings)
}

/// Extract quoted strings from a line
fn extract_strings(line: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let re = Regex::new(r#"['"]([^'"]{8,})['"]"#).unwrap();

    for cap in re.captures_iter(line) {
        if let Some(m) = cap.get(1) {
            strings.push(m.as_str().to_string());
        }
    }

    strings
}

/// Calculate Shannon entropy of a string
fn shannon_entropy(s: &str) -> f64 {
    let len = s.len() as f64;
    if len == 0.0 {
        return 0.0;
    }

    let mut freq: HashMap<char, usize> = HashMap::new();
    for c in s.chars() {
        *freq.entry(c).or_insert(0) += 1;
    }

    freq.values()
        .map(|&count| {
            let p = count as f64 / len;
            -p * p.log2()
        })
        .sum()
}

/// Check if a high-entropy string is likely a false positive
fn is_likely_false_positive(s: &str) -> bool {
    // Common non-secret patterns
    let fp_patterns = [
        // UUIDs
        Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").unwrap(),
        // Hex hashes (SHA, MD5, etc.)
        Regex::new(r"^[0-9a-fA-F]{32,}$").unwrap(),
        // Base64 encoded common strings
        Regex::new(r"^[A-Za-z0-9+/]+=*$").unwrap(),
    ];

    // Check if it matches a known false positive pattern
    for pattern in &fp_patterns {
        if pattern.is_match(s) {
            return true;
        }
    }

    // Check if it's all same character repeated
    if s.chars().collect::<std::collections::HashSet<_>>().len() <= 2 {
        return true;
    }

    // Check if it looks like a version string or date
    if s.contains('.') && s.chars().filter(|c| *c == '.').count() >= 2 {
        return true;
    }

    false
}

/// Generic pattern names eligible for placeholder filtering.
///
/// Only these broad-matching patterns can be suppressed when the matched value
/// looks like a placeholder. Specific patterns (AWS, GitHub, Slack, etc.) are
/// never suppressed because a structural match on those is high-confidence
/// regardless of the value content.
const GENERIC_PATTERN_NAMES: &[&str] = &["API Key", "Password", "Secret"];

/// Uppercase words that indicate a placeholder value rather than a real secret.
const PLACEHOLDER_WORDS: &[&str] = &[
    "YOUR_",
    "REPLACE",
    "EXAMPLE",
    "CHANGEME",
    "FIXME",
    "TODO",
    "INSERT",
    "PLACEHOLDER",
];

/// Characters that, when a value consists entirely of them (3+ chars), indicate filler.
const FILLER_CHARS: &[char] = &['x', 'X', '*', '?', '0'];

/// Check whether a pattern-based match on a line is a placeholder/example value.
///
/// Returns `true` (skip this finding) when ALL of:
/// 1. `pattern_name` is one of the generic patterns ("API Key", "Password", "Secret")
/// 2. The line contains an assignment (`=` or `:`) with a quoted value
/// 3. The value contains a placeholder indicator (uppercase keyword, angle-bracket
///    template, template marker, or repeated filler characters)
///
/// For specific patterns (AWS, GitHub, Private Key, etc.) this always returns `false`.
fn is_placeholder_pattern_match(line: &str, pattern_name: &str) -> bool {
    // Only filter generic patterns
    if !GENERIC_PATTERN_NAMES.contains(&pattern_name) {
        return false;
    }

    // Extract the value portion: find assignment operator, then quoted value
    let value = match extract_assigned_value(line) {
        Some(v) => v,
        None => return false,
    };

    let upper = value.to_uppercase();

    // Check uppercase placeholder words
    for word in PLACEHOLDER_WORDS {
        if upper.contains(word) {
            return true;
        }
    }

    // Check angle-bracket templates: <...>
    if value.contains('<') && value.contains('>') {
        return true;
    }

    // Check template markers: ${...} or {{...}}
    if value.contains("${") || value.contains("{{") {
        return true;
    }

    // Check repeated filler characters (strip non-filler chars like hyphens first)
    let stripped: String = value.chars().filter(|c| *c != '-' && *c != '_').collect();
    if stripped.len() >= 3 {
        for &filler in FILLER_CHARS {
            if stripped.chars().all(|c| c == filler) {
                return true;
            }
        }
    }

    false
}

/// Extract the value portion from an assignment line.
///
/// Looks for `=` or `:` followed by a quoted string, and returns the content
/// inside the quotes. Returns `None` if no assignment with a quoted value is found.
fn extract_assigned_value(line: &str) -> Option<String> {
    // Find the assignment operator
    let after_op = if let Some(idx) = line.find('=') {
        &line[idx + 1..]
    } else if let Some(idx) = line.find(':') {
        &line[idx + 1..]
    } else {
        return None;
    };

    // Find the first quoted string after the operator
    let trimmed = after_op.trim();
    let (quote, rest) = if let Some(stripped) = trimmed.strip_prefix('"') {
        ('"', stripped)
    } else if let Some(stripped) = trimmed.strip_prefix('\'') {
        ('\'', stripped)
    } else {
        return None;
    };

    // Find the closing quote
    rest.find(quote).map(|end| rest[..end].to_string())
}

/// Mask a secret value for safe display
fn mask_secret(value: &str) -> String {
    let len = value.len();
    if len <= 8 {
        return "*".repeat(len);
    }

    let visible = 4.min(len / 4);
    format!(
        "{}{}{}",
        &value[..visible],
        "*".repeat(len - visible * 2),
        &value[len - visible..]
    )
}

/// Truncate a line to max length
fn truncate_line(line: &str, max_len: usize) -> String {
    if line.len() <= max_len {
        line.to_string()
    } else {
        // Find nearest char boundary to avoid UTF-8 panic
        let mut end = max_len - 3;
        while end > 0 && !line.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &line[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shannon_entropy() {
        // Low entropy (repetitive)
        assert!(shannon_entropy("aaaaaaaaaa") < 1.0);

        // High entropy (random-looking)
        assert!(shannon_entropy("aB3$kL9@mN2#") > 3.0);
    }

    #[test]
    fn test_mask_secret() {
        assert_eq!(mask_secret("short"), "*****");
        assert_eq!(mask_secret("AKIAIOSFODNN7EXAMPLE"), "AKIA************MPLE");
    }

    #[test]
    fn test_aws_key_pattern() {
        let pattern = &SECRET_PATTERNS[0];
        assert!(pattern.pattern.is_match("AKIAIOSFODNN7EXAMPLE"));
        assert!(!pattern.pattern.is_match("AKIA")); // Too short
    }

    #[test]
    fn test_private_key_pattern() {
        let pattern = &SECRET_PATTERNS[2];
        assert!(pattern.pattern.is_match("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(pattern.pattern.is_match("-----BEGIN PRIVATE KEY-----"));
    }

    #[test]
    fn test_test_file_detection() {
        assert!(TEST_FILE_PATTERNS.is_match("test_secrets.py"));
        assert!(TEST_FILE_PATTERNS.is_match("secrets.test.js"));
        assert!(TEST_FILE_PATTERNS.is_match("conftest.py"));
        assert!(!TEST_FILE_PATTERNS.is_match("secrets.py"));
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
    }

    #[test]
    fn test_extract_strings() {
        let strings = extract_strings(r#"api_key = "sk-abcdefghijklmnop""#);
        assert_eq!(strings.len(), 1);
        assert_eq!(strings[0], "sk-abcdefghijklmnop");
    }

    // ---- is_placeholder_pattern_match tests ----

    #[test]
    fn test_placeholder_skips_generic_patterns_only() {
        // Generic patterns ("API Key", "Password", "Secret") should be skippable
        assert!(is_placeholder_pattern_match(
            r#"API_KEY = "YOUR_API_KEY_HERE""#,
            "API Key"
        ));
        assert!(is_placeholder_pattern_match(
            r#"password = "REPLACE_ME""#,
            "Password"
        ));
        assert!(is_placeholder_pattern_match(
            r#"SECRET_TOKEN = "<your-secret-token>""#,
            "Secret"
        ));
    }

    #[test]
    fn test_placeholder_never_skips_specific_patterns() {
        // Specific patterns must never be skipped, even if value looks like placeholder
        assert!(!is_placeholder_pattern_match(
            r#"key = "YOUR_API_KEY_HERE""#,
            "AWS Access Key"
        ));
        assert!(!is_placeholder_pattern_match(
            r#"key = "YOUR_API_KEY_HERE""#,
            "AWS Secret Key"
        ));
        assert!(!is_placeholder_pattern_match(
            r#"key = "REPLACE_ME""#,
            "GitHub Token"
        ));
        assert!(!is_placeholder_pattern_match(
            r#"key = "REPLACE_ME""#,
            "Private Key"
        ));
        assert!(!is_placeholder_pattern_match(
            r#"key = "REPLACE_ME""#,
            "Database URL"
        ));
        assert!(!is_placeholder_pattern_match(
            r#"key = "REPLACE_ME""#,
            "Slack Token"
        ));
        assert!(!is_placeholder_pattern_match(
            r#"key = "REPLACE_ME""#,
            "JWT"
        ));
        assert!(!is_placeholder_pattern_match(
            r#"key = "REPLACE_ME""#,
            "Bearer Token"
        ));
    }

    #[test]
    fn test_placeholder_uppercase_words() {
        // YOUR_ prefix
        assert!(is_placeholder_pattern_match(
            r#"api_key = "YOUR_KEY_VALUE""#,
            "API Key"
        ));
        // REPLACE
        assert!(is_placeholder_pattern_match(
            r#"secret = "REPLACE_THIS""#,
            "Secret"
        ));
        // EXAMPLE
        assert!(is_placeholder_pattern_match(
            r#"api_key = "EXAMPLE_KEY_12345""#,
            "API Key"
        ));
        // CHANGEME
        assert!(is_placeholder_pattern_match(
            r#"password = "CHANGEME""#,
            "Password"
        ));
        // FIXME
        assert!(is_placeholder_pattern_match(
            r#"token = "FIXME_token""#,
            "Secret"
        ));
        // TODO
        assert!(is_placeholder_pattern_match(
            r#"secret = "TODO_fill_this""#,
            "Secret"
        ));
        // INSERT
        assert!(is_placeholder_pattern_match(
            r#"password = "INSERT_PASSWORD""#,
            "Password"
        ));
        // PLACEHOLDER
        assert!(is_placeholder_pattern_match(
            r#"token = "PLACEHOLDER_value""#,
            "Secret"
        ));
    }

    #[test]
    fn test_placeholder_angle_bracket_templates() {
        assert!(is_placeholder_pattern_match(
            r#"password = "<password>""#,
            "Password"
        ));
        assert!(is_placeholder_pattern_match(
            r#"secret = "<your-api-key>""#,
            "Secret"
        ));
        assert!(is_placeholder_pattern_match(
            r#"token = "<insert-token-here>""#,
            "Secret"
        ));
    }

    #[test]
    fn test_placeholder_template_markers() {
        // ${...} style
        assert!(is_placeholder_pattern_match(
            r#"secret = "${SECRET_TOKEN}""#,
            "Secret"
        ));
        // {{...}} style
        assert!(is_placeholder_pattern_match(
            r#"password = "{{vault.password}}""#,
            "Password"
        ));
    }

    #[test]
    fn test_placeholder_repeated_filler_chars() {
        // All x's
        assert!(is_placeholder_pattern_match(
            r#"token = "xxx-xxx-xxx""#,
            "Secret"
        ));
        // All *'s
        assert!(is_placeholder_pattern_match(
            r#"password = "********""#,
            "Password"
        ));
        // All ?'s
        assert!(is_placeholder_pattern_match(
            r#"secret = "????????""#,
            "Secret"
        ));
        // All 0's
        assert!(is_placeholder_pattern_match(
            r#"token = "0000000000""#,
            "Secret"
        ));
        // Too short (2 chars) should not match filler
        assert!(!is_placeholder_pattern_match(
            r#"password = "xx""#,
            "Password"
        ));
    }

    #[test]
    fn test_placeholder_real_secrets_not_skipped() {
        // Real high-entropy secrets must NOT be treated as placeholders
        assert!(!is_placeholder_pattern_match(
            r#"api_key = "a3f8b2c1d4e5f6789012345678abcdef""#,
            "API Key"
        ));
        assert!(!is_placeholder_pattern_match(
            r#"password = "S3cur3P@ssw0rd!2024""#,
            "Password"
        ));
        assert!(!is_placeholder_pattern_match(
            r#"secret = "K8mPqR3sT7uVwX2yZ4aBcDeFgHjKm""#,
            "Secret"
        ));
    }

    #[test]
    fn test_placeholder_no_value_portion() {
        // Lines without = or : should not be skipped (no value to extract)
        assert!(!is_placeholder_pattern_match(
            r#"echo "YOUR_API_KEY_HERE""#,
            "API Key"
        ));
    }

    #[test]
    fn test_placeholder_scan_file_integration() {
        // Integration: scan_file should filter out placeholder findings
        use std::io::Write;
        let dir = std::env::temp_dir().join("tldr_test_placeholder");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config_template.py");
        {
            let mut f = std::fs::File::create(&file).unwrap();
            writeln!(f, r#"SECRET = "REPLACE_ME""#).unwrap();
            writeln!(f, r#"TOKEN = "xxx-xxx-xxx""#).unwrap();
            writeln!(f, r#"PASSWORD = "<password>""#).unwrap();
        }
        let findings = scan_file(&file, 4.5).unwrap();
        let secret_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "Secret" || f.pattern == "Password" || f.pattern == "API Key")
            .collect();
        assert!(
            secret_findings.is_empty(),
            "Placeholder values should produce 0 pattern findings, got {}: {:?}",
            secret_findings.len(),
            secret_findings
                .iter()
                .map(|f| format!("{}: {}", f.line, f.pattern))
                .collect::<Vec<_>>()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_real_secrets_still_detected_after_placeholder_filter() {
        // Integration: real secrets must still be found
        use std::io::Write;
        let dir = std::env::temp_dir().join("tldr_test_real_secrets");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.py");
        {
            let mut f = std::fs::File::create(&file).unwrap();
            writeln!(f, r#"secret = "K8mPqR3sT7uVwX2yZ4aBcDeFgHjKmNpQr""#).unwrap();
            writeln!(f, r#"password = "S3cur3P@ssw0rd!2024""#).unwrap();
        }
        let findings = scan_file(&file, 4.5).unwrap();
        let secret_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.pattern == "Secret" || f.pattern == "Password")
            .collect();
        assert!(
            !secret_findings.is_empty(),
            "Real secrets must still be detected after placeholder filter"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
