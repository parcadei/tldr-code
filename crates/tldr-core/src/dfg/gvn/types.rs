//! GVN Core Types
//!
//! Data structures for Global Value Numbering analysis results.

use serde::{Deserialize, Serialize};
use serde_json::json;

// =============================================================================
// Expression Reference
// =============================================================================

/// A reference to an expression in source code with its value number
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpressionRef {
    /// The source text of the expression
    pub text: String,
    /// Line number where the expression occurs (1-based)
    pub line: usize,
    /// The assigned value number
    pub value_number: usize,
}

impl ExpressionRef {
    /// Create a new expression reference
    pub fn new(text: &str, line: usize, value_number: usize) -> Self {
        Self {
            text: text.to_string(),
            line,
            value_number,
        }
    }
}

// =============================================================================
// GVN Equivalence Class
// =============================================================================

/// A group of expressions that share the same value number
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GVNEquivalence {
    /// The shared value number for this equivalence class
    pub value_number: usize,
    /// All expressions in this equivalence class
    pub expressions: Vec<ExpressionRef>,
    /// Reason why these expressions are equivalent
    /// (e.g., "commutativity", "identical expression", "alias")
    pub reason: String,
}

impl GVNEquivalence {
    /// Create a new equivalence class
    pub fn new(value_number: usize, expressions: Vec<ExpressionRef>, reason: &str) -> Self {
        Self {
            value_number,
            expressions,
            reason: reason.to_string(),
        }
    }

    /// Check if this equivalence class is non-trivial (has multiple expressions)
    pub fn is_significant(&self) -> bool {
        self.expressions.len() > 1
    }
}

// =============================================================================
// Redundancy Detection
// =============================================================================

/// A redundant expression and its original
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Redundancy {
    /// The first (original) occurrence of the expression
    pub original: ExpressionRef,
    /// The redundant (later) occurrence
    pub redundant: ExpressionRef,
    /// Reason for the redundancy
    /// (e.g., "identical expression", "commutative equivalence")
    pub reason: String,
}

impl Redundancy {
    /// Create a new redundancy record
    pub fn new(original: ExpressionRef, redundant: ExpressionRef, reason: &str) -> Self {
        Self {
            original,
            redundant,
            reason: reason.to_string(),
        }
    }
}

// =============================================================================
// GVN Report
// =============================================================================

/// Report for a single function's GVN analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GVNReport {
    /// Name of the analyzed function
    pub function: String,
    /// Equivalence classes (groups of expressions with same value number)
    pub equivalences: Vec<GVNEquivalence>,
    /// Detected redundancies (expressions that could be eliminated)
    pub redundancies: Vec<Redundancy>,
    /// Total number of expressions analyzed
    pub total_expressions: usize,
    /// Number of unique value numbers assigned
    pub unique_values: usize,
}

impl GVNReport {
    /// Create a new GVN report
    pub fn new(function: &str) -> Self {
        Self {
            function: function.to_string(),
            equivalences: Vec::new(),
            redundancies: Vec::new(),
            total_expressions: 0,
            unique_values: 0,
        }
    }

    /// Compression ratio: unique_values / total_expressions
    ///
    /// A ratio of 1.0 means no sharing (or zero expressions).
    /// A ratio of 0.5 means half the expressions share values.
    /// Lower ratios indicate more redundancy.
    pub fn compression_ratio(&self) -> f64 {
        if self.total_expressions == 0 {
            1.0
        } else {
            self.unique_values as f64 / self.total_expressions as f64
        }
    }

    /// Convert to JSON dictionary format
    pub fn to_dict(&self) -> serde_json::Value {
        json!({
            "function": self.function,
            "equivalences": self.equivalences.iter().map(|eq| {
                json!({
                    "value_number": eq.value_number,
                    "expressions": eq.expressions.iter().map(|e| {
                        json!({
                            "text": e.text,
                            "line": e.line,
                            "value_number": e.value_number
                        })
                    }).collect::<Vec<_>>(),
                    "reason": eq.reason
                })
            }).collect::<Vec<_>>(),
            "redundancies": self.redundancies.iter().map(|r| {
                json!({
                    "original": {
                        "text": r.original.text,
                        "line": r.original.line
                    },
                    "redundant": {
                        "text": r.redundant.text,
                        "line": r.redundant.line
                    },
                    "reason": r.reason
                })
            }).collect::<Vec<_>>(),
            "total_expressions": self.total_expressions,
            "unique_values": self.unique_values,
            "compression_ratio": self.compression_ratio()
        })
    }

    /// Convert to human-readable text format
    pub fn to_text(&self) -> String {
        let mut output = String::new();

        output.push_str(&format!("GVN Report: {}\n", self.function));
        output.push_str(&format!(
            "============{}\n\n",
            "=".repeat(self.function.len())
        ));

        output.push_str(&format!("Total Expressions: {}\n", self.total_expressions));
        output.push_str(&format!("Unique Values: {}\n", self.unique_values));
        output.push_str(&format!(
            "Compression Ratio: {:.2}\n\n",
            self.compression_ratio()
        ));

        if !self.equivalences.is_empty() {
            output.push_str("Equivalence Classes:\n");
            for eq in &self.equivalences {
                if eq.is_significant() {
                    output.push_str(&format!(
                        "  VN{}: {} ({} expressions, {})\n",
                        eq.value_number,
                        eq.expressions
                            .iter()
                            .map(|e| format!("\"{}\" (L{})", e.text, e.line))
                            .collect::<Vec<_>>()
                            .join(", "),
                        eq.expressions.len(),
                        eq.reason
                    ));
                }
            }
            output.push('\n');
        }

        if !self.redundancies.is_empty() {
            output.push_str("Redundancies:\n");
            for r in &self.redundancies {
                output.push_str(&format!(
                    "  \"{}\" (L{}) is redundant with \"{}\" (L{}) - {}\n",
                    r.redundant.text, r.redundant.line, r.original.text, r.original.line, r.reason
                ));
            }
        }

        output
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expression_ref_new() {
        let expr = ExpressionRef::new("x + y", 10, 1);
        assert_eq!(expr.text, "x + y");
        assert_eq!(expr.line, 10);
        assert_eq!(expr.value_number, 1);
    }

    #[test]
    fn test_gvn_equivalence_significant() {
        let single = GVNEquivalence::new(1, vec![ExpressionRef::new("x", 1, 1)], "single");
        assert!(!single.is_significant());

        let multiple = GVNEquivalence::new(
            2,
            vec![
                ExpressionRef::new("x + y", 1, 2),
                ExpressionRef::new("y + x", 2, 2),
            ],
            "commutativity",
        );
        assert!(multiple.is_significant());
    }

    #[test]
    fn test_gvn_report_compression_ratio_zero() {
        let report = GVNReport {
            function: "test".to_string(),
            equivalences: vec![],
            redundancies: vec![],
            total_expressions: 0,
            unique_values: 0,
        };
        assert_eq!(report.compression_ratio(), 1.0);
    }

    #[test]
    fn test_gvn_report_compression_ratio_full() {
        let report = GVNReport {
            function: "test".to_string(),
            equivalences: vec![],
            redundancies: vec![],
            total_expressions: 10,
            unique_values: 10,
        };
        assert_eq!(report.compression_ratio(), 1.0);
    }

    #[test]
    fn test_gvn_report_compression_ratio_half() {
        let report = GVNReport {
            function: "test".to_string(),
            equivalences: vec![],
            redundancies: vec![],
            total_expressions: 10,
            unique_values: 5,
        };
        assert_eq!(report.compression_ratio(), 0.5);
    }

    #[test]
    fn test_gvn_report_to_dict() {
        let report = GVNReport {
            function: "example".to_string(),
            equivalences: vec![GVNEquivalence::new(
                1,
                vec![
                    ExpressionRef::new("x + y", 1, 1),
                    ExpressionRef::new("y + x", 2, 1),
                ],
                "commutativity",
            )],
            redundancies: vec![Redundancy::new(
                ExpressionRef::new("x + y", 1, 1),
                ExpressionRef::new("y + x", 2, 1),
                "commutative equivalence",
            )],
            total_expressions: 2,
            unique_values: 1,
        };

        let dict = report.to_dict();
        assert_eq!(dict["function"], "example");
        assert_eq!(dict["total_expressions"], 2);
        assert_eq!(dict["unique_values"], 1);
        assert_eq!(dict["compression_ratio"], 0.5);
    }

    #[test]
    fn test_gvn_report_to_text() {
        let report = GVNReport {
            function: "example".to_string(),
            equivalences: vec![GVNEquivalence::new(
                1,
                vec![
                    ExpressionRef::new("x + y", 1, 1),
                    ExpressionRef::new("y + x", 2, 1),
                ],
                "commutativity",
            )],
            redundancies: vec![Redundancy::new(
                ExpressionRef::new("x + y", 1, 1),
                ExpressionRef::new("y + x", 2, 1),
                "commutative equivalence",
            )],
            total_expressions: 2,
            unique_values: 1,
        };

        let text = report.to_text();
        assert!(text.contains("GVN Report: example"));
        assert!(text.contains("Total Expressions: 2"));
        assert!(text.contains("Unique Values: 1"));
        assert!(text.contains("Compression Ratio: 0.50"));
        assert!(text.contains("Equivalence Classes:"));
        assert!(text.contains("Redundancies:"));
    }
}
