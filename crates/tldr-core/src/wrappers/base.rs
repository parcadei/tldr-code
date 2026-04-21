//! Base infrastructure types and functions for sub-analysis orchestration

use serde::{Deserialize, Serialize, Serializer};
use std::time::Instant;

/// Round a float to 1 decimal place for consistent serialization.
/// MIT-OUT-02a: Match Python's rounding behavior.
fn round_to_1_decimal(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Custom serializer that rounds elapsed_ms to 1 decimal place.
fn serialize_elapsed_ms<S>(elapsed_ms: &f64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_f64(round_to_1_decimal(*elapsed_ms))
}

/// Result of a single sub-analysis, capturing success/failure state, data, and timing.
///
/// # Examples
///
/// ```rust,ignore
/// use tldr_core::wrappers::SubAnalysisResult;
/// use serde_json::json;
///
/// let result = SubAnalysisResult {
///     name: "taint_analysis".to_string(),
///     success: true,
///     data: Some(json!({"flows": 3})),
///     error: None,
///     elapsed_ms: 42.5,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAnalysisResult {
    /// Name of the sub-analysis (e.g., "taint", "secrets", "gvn")
    pub name: String,

    /// Whether the analysis completed successfully
    pub success: bool,

    /// Analysis result data (if successful)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,

    /// Error message (if failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Elapsed time in milliseconds (rounded to 1 decimal place)
    /// MIT-OUT-02a: Consistent with Python's output format
    #[serde(serialize_with = "serialize_elapsed_ms")]
    pub elapsed_ms: f64,
}

/// Execute a closure safely, capturing timing and any errors.
///
/// This wraps a sub-analysis, timing its execution and catching any errors.
/// The result is always a `SubAnalysisResult` - it never panics or propagates errors.
///
/// # Arguments
///
/// * `name` - Name of the analysis for reporting
/// * `f` - Closure that performs the analysis and returns `Result<T, anyhow::Error>`
///
/// # Returns
///
/// A `SubAnalysisResult` with:
/// - `success: true` and `data: Some(...)` if the closure succeeded
/// - `success: false` and `error: Some(...)` if the closure returned an error
///
/// # Examples
///
/// ```rust,ignore
/// use tldr_core::wrappers::safe_call;
///
/// let result = safe_call("my_analysis", || {
///     Ok(42)
/// });
/// assert!(result.success);
/// ```
pub fn safe_call<F, T>(name: &str, f: F) -> SubAnalysisResult
where
    F: FnOnce() -> Result<T, anyhow::Error>,
    T: Serialize,
{
    let start = Instant::now();
    match f() {
        Ok(data) => {
            let elapsed = start.elapsed();
            SubAnalysisResult {
                name: name.to_string(),
                success: true,
                data: Some(serde_json::to_value(&data).unwrap_or(serde_json::Value::Null)),
                error: None,
                elapsed_ms: elapsed.as_secs_f64() * 1000.0,
            }
        }
        Err(e) => {
            let elapsed = start.elapsed();
            SubAnalysisResult {
                name: name.to_string(),
                success: false,
                data: None,
                error: Some(e.to_string()),
                elapsed_ms: elapsed.as_secs_f64() * 1000.0,
            }
        }
    }
}

/// Print progress message to stderr.
///
/// MIT-OUT-03a: Matches Python's exact progress format:
/// `[step/total] Analyzing {name}...`
///
/// # Arguments
///
/// * `step` - Current step number (1-indexed)
/// * `total` - Total number of steps
/// * `name` - Name of what's being analyzed
///
/// # Examples
///
/// ```rust,ignore
/// use tldr_core::wrappers::progress;
///
/// progress(1, 5, "taint_analysis");
/// // Prints: [1/5] Analyzing taint_analysis...
/// ```
pub fn progress(step: usize, total: usize, name: &str) {
    eprintln!("[{}/{}] Analyzing {}...", step, total, name);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_to_1_decimal() {
        assert_eq!(round_to_1_decimal(1.234), 1.2);
        assert_eq!(round_to_1_decimal(1.25), 1.3); // round half up
        assert_eq!(round_to_1_decimal(1.0), 1.0);
        assert_eq!(round_to_1_decimal(0.05), 0.1); // round half up
        assert_eq!(round_to_1_decimal(99.999), 100.0);
    }

    #[test]
    fn test_sub_analysis_result_default_fields() {
        let result = SubAnalysisResult {
            name: "test".to_string(),
            success: true,
            data: None,
            error: None,
            elapsed_ms: 0.0,
        };
        assert_eq!(result.name, "test");
        assert!(result.success);
    }
}
