//! Shared types for the wrappers module

use serde::{Deserialize, Serialize};

/// Severity level for findings in security and quality analyses.
///
/// Ordering is from most severe (Critical) to least severe (Info).
/// This ordering allows sorting findings by severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    /// Informational finding, no action required
    Info,
    /// Low severity, minor issue
    Low,
    /// Medium severity, should be addressed
    Medium,
    /// High severity, needs attention
    High,
    /// Critical severity, must fix immediately
    Critical,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_debug() {
        assert_eq!(format!("{:?}", Severity::Critical), "Critical");
    }

    #[test]
    fn test_severity_clone() {
        let s = Severity::High;
        let cloned = s;
        assert_eq!(s, cloned);
    }

    #[test]
    fn test_severity_copy() {
        let s = Severity::Medium;
        let copied = s; // Copy
        assert_eq!(s, copied); // Original still usable
    }
}
