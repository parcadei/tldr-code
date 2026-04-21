//! SSA Type Definitions
//!
//! Core types for SSA form representation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

// =============================================================================
// SSA Error Types
// =============================================================================

/// SSA construction and analysis errors
#[derive(Debug, Clone, Error)]
pub enum SsaError {
    /// Function not found in source
    #[error("Function not found: {0}")]
    FunctionNotFound(String),

    /// Invalid CFG structure
    #[error("Invalid CFG: {0}")]
    InvalidCfg(String),

    /// Dominator tree construction failed
    #[error("Dominator tree construction failed: {0}")]
    DominatorError(String),

    /// Phi function placement failed
    #[error("Phi placement failed: {0}")]
    PhiPlacementError(String),

    /// Variable renaming failed
    #[error("Variable renaming failed: {0}")]
    RenamingError(String),

    /// Unsupported language construct
    #[error("Unsupported language construct: {0}")]
    UnsupportedConstruct(String),

    /// Analysis limit exceeded
    #[error("Analysis limit exceeded: {0}")]
    LimitExceeded(String),

    /// No CFG available for function
    #[error("No CFG available for function {0}")]
    NoCfg(String),

    /// Empty function
    #[error("Empty function {0}")]
    EmptyFunction(String),

    /// Unreachable code detected
    #[error("Unreachable code at block {0}")]
    UnreachableCode(usize),

    /// Invalid block ID
    #[error("Invalid block ID: {0}")]
    InvalidBlockId(usize),

    /// Variable not found
    #[error("Variable not found: {0}")]
    VariableNotFound(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

// =============================================================================
// Core SSA Types
// =============================================================================

/// SSA form for a function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsaFunction {
    /// Function name
    pub function: String,
    /// File path
    pub file: PathBuf,
    /// SSA construction type used
    pub ssa_type: SsaType,
    /// SSA blocks with phi functions and instructions
    pub blocks: Vec<SsaBlock>,
    /// All SSA names (versioned variables)
    pub ssa_names: Vec<SsaName>,
    /// Def-use chains: for each SSA name, its uses
    pub def_use: HashMap<SsaNameId, Vec<SsaNameId>>,
    /// Statistics
    pub stats: SsaStats,
}

/// Type of SSA construction
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SsaType {
    /// Minimal SSA - phi functions at all merge points in iterated dominance frontier
    #[default]
    Minimal,
    /// Semi-Pruned SSA - only non-local variables get phi functions
    SemiPruned,
    /// Pruned SSA - requires liveness analysis, minimal phi functions
    Pruned,
}

/// SSA block with phi functions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsaBlock {
    /// Block ID (matches CFG block ID)
    pub id: usize,
    /// Optional label (e.g., "entry", "exit")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Line range covered by this block
    pub lines: (u32, u32),
    /// Phi functions at the start of this block
    pub phi_functions: Vec<PhiFunction>,
    /// SSA instructions in this block
    pub instructions: Vec<SsaInstruction>,
    /// Successor block IDs
    pub successors: Vec<usize>,
    /// Predecessor block IDs
    pub predecessors: Vec<usize>,
}

/// Phi function: target = phi(source_1, source_2, ...)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhiFunction {
    /// Target SSA name (the result of the phi)
    pub target: SsaNameId,
    /// Original variable name
    pub variable: String,
    /// Sources from predecessor blocks
    pub sources: Vec<PhiSource>,
    /// Line number (typically first line of block)
    pub line: u32,
}

/// Source for a phi function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhiSource {
    /// Predecessor block ID
    pub block: usize,
    /// SSA name from that predecessor
    pub name: SsaNameId,
}

/// SSA instruction (definition or expression)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsaInstruction {
    /// Instruction type
    pub kind: SsaInstructionKind,
    /// Target SSA name (if this defines a variable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<SsaNameId>,
    /// SSA names used by this instruction
    pub uses: Vec<SsaNameId>,
    /// Source line number
    pub line: u32,
    /// Original source text (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_text: Option<String>,
}

/// Kind of SSA instruction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SsaInstructionKind {
    /// Assignment: target = expr
    Assign,
    /// Parameter definition
    Param,
    /// Binary operation
    BinaryOp,
    /// Unary operation
    UnaryOp,
    /// Function call
    Call,
    /// Return statement
    Return,
    /// Branch condition
    Branch,
}

/// Unique ID for an SSA name
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SsaNameId(pub u32);

/// SSA name: versioned variable
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsaName {
    /// Unique ID
    pub id: SsaNameId,
    /// Original variable name
    pub variable: String,
    /// Version number (subscript)
    pub version: u32,
    /// Defining block (None for phi result before assignment)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub def_block: Option<usize>,
    /// Line of definition
    pub def_line: u32,
}

impl SsaName {
    /// Format as "x_1", "y_2", etc.
    pub fn format_name(&self) -> String {
        format!("{}_{}", self.variable, self.version)
    }
}

impl std::fmt::Display for SsaName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}_{}", self.variable, self.version)
    }
}

impl std::fmt::Display for SsaNameId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "${}", self.0)
    }
}

/// SSA statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SsaStats {
    /// Number of phi functions
    pub phi_count: usize,
    /// Number of unique SSA names
    pub ssa_names: usize,
    /// Number of blocks
    pub blocks: usize,
    /// Number of instructions
    pub instructions: usize,
    /// Number of dead phi functions (pruned)
    pub dead_phi_count: usize,
}

// =============================================================================
// Default Implementations
// =============================================================================

impl Default for SsaFunction {
    fn default() -> Self {
        SsaFunction {
            function: String::new(),
            file: PathBuf::new(),
            ssa_type: SsaType::Minimal,
            blocks: Vec::new(),
            ssa_names: Vec::new(),
            def_use: HashMap::new(),
            stats: SsaStats::default(),
        }
    }
}
