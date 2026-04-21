//! Def-Use and Use-Def Chains
//!
//! Enhanced types for def-use and use-def chain analysis.
//! These build on the basic reaching definitions to provide
//! explicit chain representations and uninitialized variable detection.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Re-export Confidence from dataflow for consistent use
pub use crate::dataflow::available::Confidence;

// =============================================================================
// Uncertain Definition Types
// =============================================================================

/// A variable definition pattern that was not recognized by the analysis.
///
/// Instead of silently ignoring unrecognized assignment patterns, we collect
/// them here so consumers can see what was missed and why.
///
/// # Example
///
/// ```rust,ignore
/// UncertainDef {
///     var: "x".to_string(),
///     line: 10,
///     reason: "assignment pattern not recognized for this language".to_string(),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UncertainDef {
    /// The variable name involved
    pub var: String,
    /// Source line where the uncertain definition occurs
    pub line: u32,
    /// Why the definition couldn't be confirmed
    pub reason: String,
}

// =============================================================================
// Def-Use Chain Types
// =============================================================================

/// A def-use chain: a definition and all uses it can reach
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefUseChain {
    /// The definition
    pub definition: Definition,
    /// All uses reached by this definition
    pub uses: Vec<Use>,
}

/// A variable definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Definition {
    /// Variable name
    pub var: String,
    /// Line number
    pub line: u32,
    /// Column (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// Block ID containing this definition
    pub block: usize,
    /// Source text (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_text: Option<String>,
}

/// A variable use
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Use {
    /// Line number
    pub line: u32,
    /// Column (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// Block ID containing this use
    pub block: usize,
    /// Context (surrounding code)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

// =============================================================================
// Use-Def Chain Types
// =============================================================================

/// A use-def chain: a use and all definitions that can reach it
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UseDefChain {
    /// The use site
    pub use_site: Use,
    /// Variable name
    pub var: String,
    /// All definitions reaching this use
    pub reaching_defs: Vec<Definition>,
}

// =============================================================================
// Uninitialized Variable Detection
// =============================================================================

/// Potentially uninitialized variable use
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninitializedUse {
    /// Variable name
    pub var: String,
    /// Line of use
    pub line: u32,
    /// Column
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// Block ID
    pub block: usize,
    /// Reason (e.g., "no definition reaches this use")
    pub reason: String,
    /// Severity: "error" (definitely uninitialized) or "warning" (possibly)
    pub severity: UninitSeverity,
}

/// Severity of uninitialized variable use
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UninitSeverity {
    /// Definitely uninitialized on all paths
    Definite,
    /// Possibly uninitialized on some paths
    Possible,
}

impl std::fmt::Display for UninitSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UninitSeverity::Definite => write!(f, "error"),
            UninitSeverity::Possible => write!(f, "warning"),
        }
    }
}

// =============================================================================
// Reaching Definitions Report Types
// =============================================================================

/// Complete reaching definitions report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReachingDefsReport {
    /// Function name
    pub function: String,
    /// File path
    pub file: PathBuf,
    /// Per-block IN/OUT sets
    pub blocks: Vec<BlockReachingDefs>,
    /// Def-use chains
    pub def_use_chains: Vec<DefUseChain>,
    /// Use-def chains
    pub use_def_chains: Vec<UseDefChain>,
    /// Uninitialized variable uses
    pub uninitialized: Vec<UninitializedUse>,
    /// Statistics
    pub stats: ReachingDefsStats,
    /// Definitions that couldn't be recognized by the analysis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncertain_defs: Vec<UncertainDef>,
    /// Overall confidence level for this analysis result.
    #[serde(default)]
    pub confidence: Confidence,
}

/// Reaching definitions for a single block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockReachingDefs {
    /// Block ID
    pub id: usize,
    /// Line range
    pub lines: (u32, u32),
    /// Definitions generated in this block
    pub gen: Vec<Definition>,
    /// Definitions killed by this block
    pub kill: Vec<Definition>,
    /// Definitions reaching block entry
    #[serde(rename = "in")]
    pub in_set: Vec<Definition>,
    /// Definitions available after block
    pub out: Vec<Definition>,
}

/// Statistics for reaching definitions
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReachingDefsStats {
    /// Total number of definitions
    pub definitions: usize,
    /// Total number of uses
    pub uses: usize,
    /// Number of blocks
    pub blocks: usize,
    /// Number of iterations to reach fixed point
    pub iterations: usize,
    /// Number of potentially uninitialized uses
    pub uninitialized_count: usize,
}

impl Default for ReachingDefsReport {
    fn default() -> Self {
        ReachingDefsReport {
            function: String::new(),
            file: PathBuf::new(),
            blocks: Vec::new(),
            def_use_chains: Vec::new(),
            use_def_chains: Vec::new(),
            uninitialized: Vec::new(),
            stats: ReachingDefsStats::default(),
            uncertain_defs: Vec::new(),
            confidence: Confidence::default(),
        }
    }
}
