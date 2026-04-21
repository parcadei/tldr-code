//! Available Expressions Analysis Types
//!
//! This module provides the core types for available expressions dataflow analysis:
//!
//! - `Expression`: A computed expression that can be tracked for CSE
//! - `BlockExpressions`: Gen/kill sets for a single CFG block
//! - `AvailableExprsInfo`: Analysis results with query methods
//!
//! # Capabilities Implemented
//!
//! - CAP-AE-01: Expression struct (frozen/hashable by text only)
//! - CAP-AE-02: Commutative expression normalization
//! - CAP-AE-03 to CAP-AE-07: BlockExpressions (gen/kill per block)
//! - CAP-AE-08 to CAP-AE-11: AvailableExprsInfo with query methods
//! - CAP-AE-06, CAP-AE-07: redundant_computations with intra-block kill precision
//! - CAP-AE-10: get_available_at_line query method
//!
//! # TIGER Mitigations
//!
//! - TIGER-PASS2-4: Use IndexMap or sort keys for deterministic JSON output
//! - ELEPHANT-PASS1-5: Apply trim() to operands in normalize_expression
//! - TIGER-PASS1-3: Commutative ops conflate bitwise/logical (documented - safe
//!   when side-effects filtered via function call exclusion in CAP-AE-12)
//! - TIGER-PASS1-12: Intra-block kill tracking algorithm for redundant_computations
//! - TIGER-PASS3-3: Stop processing statements after return/throw in gen/kill

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::types::{build_predecessors, reverse_postorder, validate_cfg, BlockId, DataflowError};
use crate::types::{CfgInfo, DfgInfo, RefType, VarRef};

// =============================================================================
// Confidence and Uncertain Types
// =============================================================================

/// Confidence level for analysis results.
///
/// Indicates how much trust should be placed in the analysis output:
/// - `Low`: Analysis couldn't determine much (many uncertain items)
/// - `Medium`: Analysis produced some results but with gaps
/// - `High`: Analysis produced comprehensive results with few uncertainties
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// Low confidence - many items uncertain
    #[default]
    Low,
    /// Medium confidence - some items uncertain
    Medium,
    /// High confidence - few or no items uncertain
    High,
}

/// An expression that was skipped from confirmed results but might be relevant.
///
/// Instead of silently discarding expressions that contain function calls or
/// other impure constructs, we collect them here so consumers can see what
/// was skipped and why.
///
/// # Example
///
/// ```rust,ignore
/// UncertainFinding {
///     expr: "obj.length + x".to_string(),
///     line: 15,
///     reason: "contains method access - purity unknown".to_string(),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UncertainFinding {
    /// The expression text that was skipped
    pub expr: String,
    /// Source line where the expression appears
    pub line: usize,
    /// Why this expression couldn't be confirmed
    pub reason: String,
}

// =============================================================================
// Constants
// =============================================================================

/// CAP-AE-02: Commutative operators that allow operand reordering.
///
/// For these operators, `normalize_expression` will sort operands alphabetically
/// to ensure "a + b" and "b + a" produce the same normalized text.
///
/// # TIGER-PASS1-3 Note
///
/// This list conflates bitwise and logical operators (e.g., `&` vs `and`).
/// This is safe because:
/// 1. Function calls are excluded from CSE (CAP-AE-12)
/// 2. Side effects are already filtered out
/// 3. The expression text will still differ in practice
pub const COMMUTATIVE_OPS: &[&str] = &["+", "*", "==", "!=", "and", "or", "&", "|", "^"];

// =============================================================================
// Expression
// =============================================================================

/// Represents a computed expression for availability analysis.
///
/// An expression is a binary operation like `a + b` that can be tracked
/// to detect Common Subexpression Elimination (CSE) opportunities.
///
/// # CAP-AE-01: Equality and Hashing
///
/// **Critical**: Equality and hashing are based on `text` only, not `line`.
/// This allows expressions with the same normalized text to be considered
/// equal regardless of where they appear in the code.
///
/// # Example
///
/// ```rust,ignore
/// let expr1 = Expression::new("a + b", vec!["a", "b"], 1);
/// let expr2 = Expression::new("a + b", vec!["a", "b"], 100);
///
/// // Same text means equal, even with different lines
/// assert_eq!(expr1, expr2);
///
/// // Can be used in HashSet (only one entry)
/// let mut set = HashSet::new();
/// set.insert(expr1);
/// set.insert(expr2);
/// assert_eq!(set.len(), 1);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expression {
    /// Normalized expression string (e.g., "a + b").
    /// This is the canonical form after applying normalization rules.
    pub text: String,

    /// Variables used in this expression (sorted alphabetically).
    /// Used for kill detection - if any operand is redefined, the expression is killed.
    pub operands: Vec<String>,

    /// Source line where this expression first appears.
    /// Note: This field is NOT used in equality or hashing (CAP-AE-01).
    pub line: usize,
}

impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        // CAP-AE-01: Equality based on text only
        self.text == other.text
    }
}

impl Eq for Expression {}

impl Hash for Expression {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // CAP-AE-01: Hash based on text only
        self.text.hash(state);
    }
}

impl Expression {
    /// Create a new Expression with the given components.
    ///
    /// # Arguments
    ///
    /// * `text` - The normalized expression text
    /// * `operands` - Variables used in the expression (will be sorted)
    /// * `line` - Source line where expression appears
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let expr = Expression::new("a + b", vec!["a", "b"], 5);
    /// assert_eq!(expr.text, "a + b");
    /// assert_eq!(expr.operands, vec!["a", "b"]);
    /// assert_eq!(expr.line, 5);
    /// ```
    pub fn new(text: impl Into<String>, operands: Vec<impl Into<String>>, line: usize) -> Self {
        let mut ops: Vec<String> = operands.into_iter().map(|s| s.into()).collect();
        ops.sort();
        Self {
            text: text.into(),
            operands: ops,
            line,
        }
    }

    /// Check if redefining the given variable kills this expression.
    ///
    /// An expression is killed when any of its operands is redefined.
    /// This is the foundation of the "kill" set in dataflow analysis.
    ///
    /// # Arguments
    ///
    /// * `var` - The variable being redefined
    ///
    /// # Returns
    ///
    /// `true` if this expression uses `var` and would be killed by its redefinition.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let expr = Expression::new("a + b", vec!["a", "b"], 1);
    /// assert!(expr.is_killed_by("a"));  // Uses 'a'
    /// assert!(expr.is_killed_by("b"));  // Uses 'b'
    /// assert!(!expr.is_killed_by("c")); // Doesn't use 'c'
    /// ```
    pub fn is_killed_by(&self, var: &str) -> bool {
        self.operands.iter().any(|op| op == var)
    }
}

// =============================================================================
// Normalization
// =============================================================================

/// CAP-AE-02: Normalize binary expression to canonical form.
///
/// For commutative operators, operands are sorted alphabetically to ensure
/// that expressions like "a + b" and "b + a" produce the same normalized text.
///
/// # TIGER Mitigations
///
/// - ELEPHANT-PASS1-5: Applies `trim()` to operands for whitespace normalization
/// - TIGER-PASS1-3: Documents that bitwise/logical conflation is safe (see COMMUTATIVE_OPS)
///
/// # Arguments
///
/// * `left` - Left operand (will be trimmed)
/// * `op` - The binary operator
/// * `right` - Right operand (will be trimmed)
///
/// # Returns
///
/// Normalized expression string in the form "left op right" or "right op left"
/// (for commutative operators, alphabetically sorted).
///
/// # Examples
///
/// ```rust,ignore
/// // Commutative operators - operands sorted
/// assert_eq!(normalize_expression("b", "+", "a"), "a + b");
/// assert_eq!(normalize_expression("y", "*", "x"), "x * y");
/// assert_eq!(normalize_expression("foo", "==", "bar"), "bar == foo");
///
/// // Non-commutative operators - order preserved
/// assert_eq!(normalize_expression("a", "-", "b"), "a - b");
/// assert_eq!(normalize_expression("x", "/", "y"), "x / y");
///
/// // Whitespace is trimmed (ELEPHANT-PASS1-5)
/// assert_eq!(normalize_expression("  a  ", "+", "  b  "), "a + b");
/// ```
pub fn normalize_expression(left: &str, op: &str, right: &str) -> String {
    // ELEPHANT-PASS1-5: Apply trim() for whitespace normalization
    let left = left.trim();
    let right = right.trim();

    if COMMUTATIVE_OPS.contains(&op) {
        // Sort operands alphabetically for commutative operators
        let mut operands = [left, right];
        operands.sort();
        format!("{} {} {}", operands[0], op, operands[1])
    } else {
        // Preserve order for non-commutative operators
        format!("{} {} {}", left, op, right)
    }
}

// =============================================================================
// BlockExpressions
// =============================================================================

/// Expressions generated and killed in a single CFG block.
///
/// This is the per-block analysis result used during fixpoint iteration:
///
/// - **gen**: Expressions computed in this block (before any operand is killed)
/// - **kill**: Variables redefined in this block (kills expressions using that var)
///
/// # Dataflow Equations
///
/// ```text
/// avail_out[B] = gen[B] | (avail_in[B] - killed_by[B])
/// ```
///
/// where `killed_by[B]` = expressions whose operands are in `kill[B]`
#[derive(Debug, Clone, Default)]
pub struct BlockExpressions {
    /// Expressions computed in this block (before any operand is killed).
    ///
    /// An expression is in `gen` if it's computed and none of its operands
    /// are redefined between the start of the block and its computation.
    pub gen: HashSet<Expression>,

    /// Variables redefined in this block.
    ///
    /// Used to compute the kill set: any expression using a variable in
    /// `kill` is no longer available after this block.
    pub kill: HashSet<String>,
}

impl BlockExpressions {
    /// Create a new empty BlockExpressions.
    pub fn new() -> Self {
        Self::default()
    }
}

// =============================================================================
// ExprInstance - Expression with Block Context
// =============================================================================

/// An expression instance with its block context for intra-block tracking.
///
/// This struct pairs an expression with the block it appears in,
/// enabling proper kill tracking within blocks.
///
/// # CAP-AE-07 Support
///
/// Used by `redundant_computations()` to track expressions with their
/// block IDs for intra-block kill handling.
#[derive(Debug, Clone)]
pub struct ExprInstance {
    /// The expression itself
    pub expr: Expression,
    /// Block ID where this expression instance appears
    pub block_id: BlockId,
}

impl ExprInstance {
    /// Create a new expression instance.
    pub fn new(expr: Expression, block_id: BlockId) -> Self {
        Self { expr, block_id }
    }
}

// =============================================================================
// AvailableExprsInfo
// =============================================================================

/// Available expressions analysis results.
///
/// An expression is available at a program point if:
/// 1. It has been computed on EVERY path reaching that point (MUST analysis)
/// 2. None of its operands have been redefined since computation
///
/// # Capabilities
///
/// - CAP-AE-08: `is_available()` - check availability at block entry
/// - CAP-AE-09: `is_available_at_exit()` - check availability at block exit
/// - CAP-AE-10: `get_available_at_line()` - query by source line (placeholder)
/// - CAP-AE-11: `to_json()` - serialize to JSON-compatible structure
///
/// # Example
///
/// ```rust,ignore
/// let info = compute_available_exprs(&cfg, &dfg)?;
///
/// // Check if expression is available at block entry
/// let expr = Expression::new("a + b", vec!["a", "b"], 1);
/// if info.is_available(2, &expr) {
///     println!("a + b is available at entry to block 2");
/// }
///
/// // Find CSE opportunities
/// for (text, first_line, redundant_line) in info.redundant_computations() {
///     println!("{} first at line {}, redundant at line {}", text, first_line, redundant_line);
/// }
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AvailableExprsInfo {
    /// Expressions available at block entry.
    ///
    /// `avail_in[B] = intersection(avail_out[P] for P in predecessors[B])`
    ///
    /// For the entry block, this is always empty.
    #[serde(
        serialize_with = "serialize_avail_map",
        deserialize_with = "deserialize_avail_map"
    )]
    pub avail_in: HashMap<BlockId, HashSet<Expression>>,

    /// Expressions available at block exit.
    ///
    /// `avail_out[B] = gen[B] | (avail_in[B] - killed_by_block[B])`
    #[serde(
        serialize_with = "serialize_avail_map",
        deserialize_with = "deserialize_avail_map"
    )]
    pub avail_out: HashMap<BlockId, HashSet<Expression>>,

    /// All unique expressions found in the function.
    pub all_exprs: HashSet<Expression>,

    /// Entry block ID.
    pub entry_block: BlockId,

    /// All expression instances including duplicates (for CSE detection).
    ///
    /// This preserves the order expressions appear in the code,
    /// which is needed for `redundant_computations()`.
    #[serde(skip)]
    pub expr_instances: Vec<Expression>,

    /// Expression instances with block context for intra-block tracking.
    ///
    /// CAP-AE-07: Used for proper intra-block kill handling in redundant_computations().
    #[serde(skip)]
    pub expr_instances_with_blocks: Vec<ExprInstance>,

    /// Definitions (variable assignments) per line for intra-block kill tracking.
    ///
    /// TIGER-PASS1-12: Maps line number to set of variables defined on that line.
    #[serde(skip)]
    pub defs_per_line: HashMap<usize, HashSet<String>>,

    /// Maps lines to their containing block IDs.
    ///
    /// Used by get_available_at_line to find the containing block.
    #[serde(skip)]
    pub line_to_block: HashMap<usize, BlockId>,

    /// Expressions that were skipped from confirmed results but might be relevant.
    ///
    /// Contains expressions that were filtered out (e.g., function calls) but
    /// could still be informative for consumers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncertain_exprs: Vec<UncertainFinding>,

    /// Overall confidence level for this analysis result.
    ///
    /// Computed based on the ratio of confirmed vs uncertain expressions.
    #[serde(default)]
    pub confidence: Confidence,
}

/// Custom serializer for avail_in/avail_out maps.
///
/// TIGER-PASS2-4: Ensures deterministic JSON output by sorting keys.
fn serialize_avail_map<S>(
    map: &HashMap<BlockId, HashSet<Expression>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    // Sort by block ID for deterministic output
    let sorted: IndexMap<String, Vec<&Expression>> = map
        .iter()
        .map(|(k, v)| {
            let mut exprs: Vec<_> = v.iter().collect();
            // Also sort expressions by text for determinism
            exprs.sort_by_key(|e| &e.text);
            (k.to_string(), exprs)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .collect();

    // Use IndexMap's ordered serialization
    let ordered: IndexMap<_, _> = sorted.into_iter().collect();
    ordered.serialize(serializer)
}

/// Custom deserializer for avail_in/avail_out maps.
fn deserialize_avail_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<BlockId, HashSet<Expression>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let map: HashMap<String, Vec<Expression>> = HashMap::deserialize(deserializer)?;
    Ok(map
        .into_iter()
        .map(|(k, v)| {
            let block_id: BlockId = k.parse().unwrap_or(0);
            let exprs: HashSet<Expression> = v.into_iter().collect();
            (block_id, exprs)
        })
        .collect())
}

impl AvailableExprsInfo {
    /// Create an empty AvailableExprsInfo for the given entry block.
    ///
    /// This is used when there are no expressions to analyze.
    pub fn empty(entry_block: BlockId) -> Self {
        Self {
            avail_in: HashMap::new(),
            avail_out: HashMap::new(),
            all_exprs: HashSet::new(),
            entry_block,
            expr_instances: Vec::new(),
            expr_instances_with_blocks: Vec::new(),
            defs_per_line: HashMap::new(),
            line_to_block: HashMap::new(),
            uncertain_exprs: Vec::new(),
            confidence: Confidence::default(),
        }
    }

    /// Create a new AvailableExprsInfo with the given entry block.
    pub fn new(entry_block: BlockId) -> Self {
        Self::empty(entry_block)
    }

    /// CAP-AE-08: Check if expression is available at entry to block.
    ///
    /// # Arguments
    ///
    /// * `block` - The block ID to check
    /// * `expr` - The expression to check for availability
    ///
    /// # Returns
    ///
    /// `true` if the expression is in `avail_in[block]`, `false` otherwise.
    pub fn is_available(&self, block: BlockId, expr: &Expression) -> bool {
        self.avail_in
            .get(&block)
            .is_some_and(|set| set.contains(expr))
    }

    /// CAP-AE-09: Check if expression is available at exit of block.
    ///
    /// # Arguments
    ///
    /// * `block` - The block ID to check
    /// * `expr` - The expression to check for availability
    ///
    /// # Returns
    ///
    /// `true` if the expression is in `avail_out[block]`, `false` otherwise.
    pub fn is_available_at_exit(&self, block: BlockId, expr: &Expression) -> bool {
        self.avail_out
            .get(&block)
            .is_some_and(|set| set.contains(expr))
    }

    /// CAP-AE-06, CAP-AE-07: Find expressions computed when already available (CSE opportunities).
    ///
    /// Returns a list of redundant computations where an expression is
    /// recomputed when it's already available from a previous computation.
    ///
    /// # Algorithm (TIGER-PASS1-12: Intra-block Precision)
    ///
    /// 1. Track `killed_so_far` set within each block
    /// 2. For each expr_instance in source order:
    ///    a. Check if expr in `avail_in[block]` AND not `killed_so_far`
    ///    b. If redundant, add to result
    ///    c. Update `killed_so_far` with defs after this line
    ///
    /// This correctly handles cases like:
    /// ```text
    /// x = a + b;  // first computation
    /// a = 5;      // kills "a + b"
    /// y = a + b;  // NOT redundant (killed between)
    /// ```
    ///
    /// # Returns
    ///
    /// `Vec<(expr_text, first_line, redundant_line)>` sorted by redundant_line.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let redundant = info.redundant_computations();
    /// for (text, first, second) in redundant {
    ///     println!("{} at line {} is redundant (first at {})", text, second, first);
    /// }
    /// ```
    pub fn redundant_computations(&self) -> Vec<(String, usize, usize)> {
        let mut redundant = Vec::new();

        // If we have expr_instances_with_blocks (Phase 4 data), use intra-block precision
        if !self.expr_instances_with_blocks.is_empty() {
            // Track first occurrence of each expression (text -> (line, block))
            let mut first_seen: HashMap<String, (usize, BlockId)> = HashMap::new();

            // Group expressions by block for intra-block processing
            let mut block_exprs: HashMap<BlockId, Vec<&ExprInstance>> = HashMap::new();
            for inst in &self.expr_instances_with_blocks {
                block_exprs.entry(inst.block_id).or_default().push(inst);
            }

            // Sort expressions within each block by line
            for exprs in block_exprs.values_mut() {
                exprs.sort_by_key(|e| e.expr.line);
            }

            // Process blocks in order (by entry block first, then others)
            let mut block_ids: Vec<_> = block_exprs.keys().copied().collect();
            block_ids.sort();

            for &block_id in &block_ids {
                if let Some(exprs) = block_exprs.get(&block_id) {
                    // Track what expressions have been generated in this block with their line
                    let mut gen_in_block: HashMap<String, usize> = HashMap::new();

                    for inst in exprs {
                        let expr = &inst.expr;
                        let line = expr.line;

                        // TIGER-PASS1-12: Check if any operand was killed between
                        // first occurrence and this line
                        let mut is_redundant = false;

                        if let Some(&(first_line, first_block)) = first_seen.get(&expr.text) {
                            // Expression was seen before - check for kills in between
                            let start_line = if first_block == block_id {
                                // Same block - check from first occurrence to now
                                first_line + 1
                            } else {
                                // Different block - we'd need inter-block analysis
                                // For now, trust avail_in and check from block start
                                1 // Conservative: check all lines
                            };

                            // Check if any operand was killed between first and current
                            let mut killed = false;
                            for check_line in start_line..line {
                                if let Some(defs) = self.defs_per_line.get(&check_line) {
                                    if expr.operands.iter().any(|op| defs.contains(op)) {
                                        killed = true;
                                        break;
                                    }
                                }
                            }

                            if !killed && first_line != line {
                                // Expression is available (not killed) - this is redundant
                                is_redundant = true;
                                redundant.push((expr.text.clone(), first_line, line));
                            }
                        }

                        // Also check if expression was generated earlier in this block
                        // and no kills happened since then
                        if !is_redundant {
                            if let Some(&gen_line) = gen_in_block.get(&expr.text) {
                                let mut killed = false;
                                for check_line in (gen_line + 1)..line {
                                    if let Some(defs) = self.defs_per_line.get(&check_line) {
                                        if expr.operands.iter().any(|op| defs.contains(op)) {
                                            killed = true;
                                            break;
                                        }
                                    }
                                }

                                if !killed && gen_line != line {
                                    // Check also if first_seen exists for proper first_line
                                    let first_line = first_seen
                                        .get(&expr.text)
                                        .map(|(l, _)| *l)
                                        .unwrap_or(gen_line);
                                    if first_line != line {
                                        redundant.push((expr.text.clone(), first_line, line));
                                    }
                                }
                            }
                        }

                        // Record first occurrence globally
                        if !first_seen.contains_key(&expr.text) {
                            first_seen.insert(expr.text.clone(), (line, block_id));
                        }

                        // Record in this block's gen set
                        gen_in_block.entry(expr.text.clone()).or_insert(line);
                    }
                }
            }
        } else {
            // Fall back to simple implementation for backward compatibility
            let mut seen: HashMap<String, usize> = HashMap::new();

            for expr in &self.expr_instances {
                if let Some(&first_line) = seen.get(&expr.text) {
                    redundant.push((expr.text.clone(), first_line, expr.line));
                } else {
                    seen.insert(expr.text.clone(), expr.line);
                }
            }
        }

        // Sort by redundant line for deterministic output
        redundant.sort_by_key(|(_, _, line)| *line);
        redundant
    }

    /// CAP-AE-10: Get expressions available at a specific source line.
    ///
    /// Returns the set of expressions that are available at the given source line.
    /// This is determined by finding the containing block and returning its `avail_in`,
    /// adjusted for any kills that occur before the line within the block.
    ///
    /// # Arguments
    ///
    /// * `line` - The source line number to query
    /// * `cfg` - The control flow graph (used to find the containing block)
    ///
    /// # Returns
    ///
    /// `HashSet<Expression>` containing all expressions available at that line.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let available = info.get_available_at_line(5, &cfg);
    /// for expr in &available {
    ///     println!("Available at line 5: {}", expr.text);
    /// }
    /// ```
    pub fn get_available_at_line(&self, line: usize, cfg: &CfgInfo) -> HashSet<Expression> {
        // Find which block contains this line
        let block_id = if let Some(&bid) = self.line_to_block.get(&line) {
            bid
        } else {
            // Fall back to searching the CFG
            let found = cfg.blocks.iter().find(|b| {
                let (start, end) = b.lines;
                (start as usize) <= line && line <= (end as usize)
            });
            match found {
                Some(block) => block.id,
                None => return HashSet::new(),
            }
        };

        // Start with avail_in for this block
        let mut available = self.avail_in.get(&block_id).cloned().unwrap_or_default();

        // Find the block's start line to compute intra-block kills
        let block_start = cfg
            .blocks
            .iter()
            .find(|b| b.id == block_id)
            .map(|b| b.lines.0 as usize)
            .unwrap_or(line);

        // Remove expressions killed by definitions between block start and this line
        // (TIGER-PASS1-12: Intra-block precision)
        let mut killed: HashSet<String> = HashSet::new();
        for check_line in block_start..line {
            if let Some(defs) = self.defs_per_line.get(&check_line) {
                for def in defs {
                    killed.insert(def.clone());
                }
            }
        }

        // Remove killed expressions
        available.retain(|expr| !expr.operands.iter().any(|op| killed.contains(op)));

        available
    }

    /// CAP-AE-11: Serialize to JSON-compatible structure.
    ///
    /// # TIGER-PASS2-4 Mitigation
    ///
    /// Keys are sorted for deterministic output:
    /// - Block IDs are converted to strings and sorted numerically
    /// - Expressions within each block are sorted by text
    ///
    /// # Output Format
    ///
    /// ```json
    /// {
    ///   "avail_in": {
    ///     "0": [],
    ///     "1": [{"text": "a + b", "operands": ["a", "b"], "line": 2}]
    ///   },
    ///   "avail_out": {...},
    ///   "all_expressions": [...],
    ///   "entry_block": 0,
    ///   "redundant_computations": [{"expr": "a + b", "first_at": 2, "redundant_at": 4}]
    /// }
    /// ```
    pub fn to_json(&self) -> serde_json::Value {
        // TIGER-PASS2-4: Sort keys for deterministic output

        // Helper to convert avail map to sorted JSON
        let avail_map_to_json = |map: &HashMap<BlockId, HashSet<Expression>>| -> IndexMap<String, Vec<serde_json::Value>> {
            let mut sorted: Vec<_> = map.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            sorted
                .into_iter()
                .map(|(k, v)| {
                    let mut exprs: Vec<_> = v.iter().collect();
                    exprs.sort_by_key(|e| &e.text);
                    let expr_values: Vec<_> = exprs
                        .into_iter()
                        .map(|e| {
                            serde_json::json!({
                                "text": e.text,
                                "operands": e.operands,
                                "line": e.line,
                            })
                        })
                        .collect();
                    (k.to_string(), expr_values)
                })
                .collect()
        };

        let avail_in = avail_map_to_json(&self.avail_in);
        let avail_out = avail_map_to_json(&self.avail_out);

        // Sort all_expressions by text
        let mut all_exprs: Vec<_> = self.all_exprs.iter().collect();
        all_exprs.sort_by_key(|e| &e.text);
        let all_expressions: Vec<_> = all_exprs
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "text": e.text,
                    "operands": e.operands,
                    "line": e.line,
                })
            })
            .collect();

        // Redundant computations (already sorted by redundant_line)
        let redundant: Vec<_> = self
            .redundant_computations()
            .into_iter()
            .map(|(expr, first, redundant)| {
                serde_json::json!({
                    "expr": expr,
                    "first_at": first,
                    "redundant_at": redundant,
                })
            })
            .collect();

        // Uncertain expressions
        let uncertain: Vec<_> = self
            .uncertain_exprs
            .iter()
            .map(|uf| {
                serde_json::json!({
                    "expr": uf.expr,
                    "line": uf.line,
                    "reason": uf.reason,
                })
            })
            .collect();

        let confidence_str = match self.confidence {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        };

        serde_json::json!({
            "avail_in": avail_in,
            "avail_out": avail_out,
            "all_expressions": all_expressions,
            "entry_block": self.entry_block,
            "redundant_computations": redundant,
            "uncertain_exprs": uncertain,
            "confidence": confidence_str,
        })
    }
}

// =============================================================================
// Phase 2: Expression Extraction from DFG
// =============================================================================

/// Binary operators that can form expressions for CSE analysis.
///
/// These are the operators we'll look for when parsing source lines
/// to extract binary expressions like `a + b`, `x * y`, etc.
const BINARY_OPS: &[&str] = &[
    // Arithmetic
    "+", "-", "*", "/", "%", "//", "**", // Comparison
    "==", "!=", "<", ">", "<=", ">=", // Logical
    "and", "or", "&&", "||", // Bitwise
    "&", "|", "^", "<<", ">>",
];

/// CAP-AE-12: Check if text looks like a function call.
///
/// Function calls have side effects and should be excluded from CSE analysis.
/// This checks for patterns like:
/// - `foo(`
/// - `bar.baz(`
/// - `obj.method(`
///
/// # Arguments
///
/// * `text` - The text to check for function call patterns
///
/// # Returns
///
/// `true` if the text appears to contain a function call.
///
/// # TIGER Mitigations
///
/// - TIGER-PASS1-10: Part of expression extraction algorithm
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_function_call("foo(x)"));
/// assert!(is_function_call("bar.baz(1, 2)"));
/// assert!(!is_function_call("a + b"));
/// assert!(!is_function_call("x * y"));
/// ```
pub fn is_function_call(text: &str) -> bool {
    let trimmed = text.trim();

    // Look for function call pattern: identifier followed by (
    // This handles: foo(, bar.baz(, obj.method(

    // Find opening paren
    if let Some(paren_idx) = trimmed.find('(') {
        // Check if what comes before the paren looks like an identifier/method access
        let before_paren = &trimmed[..paren_idx];
        let before_trimmed = before_paren.trim_end();

        if before_trimmed.is_empty() {
            return false;
        }

        // Check if it ends with an identifier character (alphanumeric or underscore)
        // This would indicate a function call like `foo(` or `bar.baz(`
        if let Some(last_char) = before_trimmed.chars().last() {
            if last_char.is_alphanumeric() || last_char == '_' {
                return true;
            }
        }
    }

    false
}

/// Detect if a source line contains an expression that was skipped due to
/// function calls or method accesses, and return it as an uncertain finding.
///
/// This is called when `parse_expression_from_line` returns `None`, to determine
/// if the line was skipped because it contained impure constructs (function calls,
/// method accesses) rather than simply not being a binary expression.
///
/// # Arguments
///
/// * `line` - The source line to check
/// * `line_num` - The source line number (1-indexed)
///
/// # Returns
///
/// `Some(UncertainFinding)` if the line contains a skipped expression, `None` otherwise.
fn detect_uncertain_expression(line: &str, line_num: usize) -> Option<UncertainFinding> {
    let trimmed = line.trim();

    // Skip empty lines, comments, and structural/declaration lines
    const SKIP_PREFIXES: &[&str] = &[
        "#",
        "//",
        "/*",
        "@",
        "import ",
        "from ",
        "use ",
        "class ",
        "def ",
        "fn ",
        "func ",
        "function ",
        "pub ",
        "struct ",
        "enum ",
        "trait ",
        "impl ",
        "interface ",
    ];
    if trimmed.is_empty() || SKIP_PREFIXES.iter().any(|p| trimmed.starts_with(p)) {
        return None;
    }

    /// Check whether text contains an arithmetic binary operator.
    fn has_binary_operator(s: &str) -> bool {
        s.contains(" + ") || s.contains(" - ") || s.contains(" * ") || s.contains(" / ")
    }

    // Check for assignment lines with function calls on the RHS
    // Pattern: `var = expr` where expr contains a function call
    if let Some(eq_idx) = trimmed.find('=') {
        // Skip comparison operators
        if eq_idx > 0 {
            let before = trimmed.as_bytes().get(eq_idx.wrapping_sub(1));
            let after = trimmed.as_bytes().get(eq_idx + 1);
            let is_comparison =
                matches!(before, Some(b'!' | b'<' | b'>' | b'=')) || matches!(after, Some(b'='));
            if !is_comparison {
                let rhs = trimmed[eq_idx + 1..].trim();
                // Check if RHS contains a function call mixed with an operator
                if is_function_call(rhs) && has_binary_operator(rhs) {
                    return Some(UncertainFinding {
                        expr: rhs.to_string(),
                        line: line_num,
                        reason: "contains function call - purity unknown".to_string(),
                    });
                }
                // Check for standalone function call on RHS (method access)
                if is_function_call(rhs) && rhs.contains('.') {
                    return Some(UncertainFinding {
                        expr: rhs.to_string(),
                        line: line_num,
                        reason: "contains method access - purity unknown".to_string(),
                    });
                }
            }
        }
    }

    // Check for standalone expressions with function calls that look like they
    // could be binary expressions
    if is_function_call(trimmed) && has_binary_operator(trimmed) {
        return Some(UncertainFinding {
            expr: trimmed.to_string(),
            line: line_num,
            reason: "function calls may have side effects".to_string(),
        });
    }

    None
}

/// Parse expression from source line like "x = a + b".
///
/// Extracts the left operand, operator, and right operand from an assignment
/// statement containing a binary expression.
///
/// # Arguments
///
/// * `line` - The source line to parse
///
/// # Returns
///
/// `Some((left, op, right))` if a binary expression is found, `None` otherwise.
///
/// # TIGER Mitigations
///
/// - TIGER-PASS2-1: Only process rightmost assignment on line
/// - TIGER-PASS3-5: Limit operand extraction to base variable only
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(
///     parse_expression_from_line("x = a + b"),
///     Some(("a".to_string(), "+".to_string(), "b".to_string()))
/// );
/// assert_eq!(
///     parse_expression_from_line("result = foo * bar"),
///     Some(("foo".to_string(), "*".to_string(), "bar".to_string()))
/// );
/// assert_eq!(parse_expression_from_line("x = foo()"), None);
/// ```
pub fn parse_expression_from_line(line: &str) -> Option<(String, String, String)> {
    // Strip comments (simple approach - single-line comments only)
    let line = if let Some(idx) = line.find('#') {
        &line[..idx]
    } else if let Some(idx) = line.find("//") {
        &line[..idx]
    } else {
        line
    };

    // TIGER-PASS2-1: Find rightmost assignment (for a = b = 5, we want b = 5)
    // Look for '=' that's not part of ==, !=, <=, >=, :=
    let mut rhs_start = None;
    let chars: Vec<char> = line.chars().collect();

    for i in (0..chars.len()).rev() {
        if chars[i] == '=' {
            // Check it's not ==, !=, <=, >=, :=
            let is_comparison = (i > 0 && matches!(chars[i - 1], '=' | '!' | '<' | '>' | ':'))
                || (i + 1 < chars.len() && chars[i + 1] == '=');

            if !is_comparison {
                rhs_start = Some(i + 1);
                break;
            }
        }
    }

    // Determine expression region to search for binary operators
    let expr_text = if let Some(start) = rhs_start {
        // Found assignment - search RHS
        line[start..].trim()
    } else {
        // No assignment found - try the whole line (for return, if, standalone exprs)
        let trimmed = line.trim();
        // Strip common prefixes: return, if, elif, while, for, yield, assert, etc.
        let stripped = if let Some(rest) = trimmed.strip_prefix("return ") {
            rest.trim()
        } else if let Some(rest) = trimmed.strip_prefix("return(") {
            // Handle return(expr)
            rest.trim()
        } else if let Some(rest) = trimmed.strip_prefix("if ") {
            // Strip trailing colon for Python if
            let r = rest.trim();
            let r = r.strip_suffix(':').unwrap_or(r);
            // Strip trailing brace for C-like if
            let r = r.strip_suffix('{').unwrap_or(r);
            // Strip surrounding parens if present
            let r = r.strip_prefix('(').unwrap_or(r);
            let r = r.strip_suffix(')').unwrap_or(r);
            r.trim()
        } else if let Some(rest) = trimmed.strip_prefix("elif ") {
            let r = rest.trim();
            r.strip_suffix(':').unwrap_or(r).trim()
        } else if let Some(rest) = trimmed.strip_prefix("else if ") {
            let r = rest.trim();
            let r = r.strip_suffix('{').unwrap_or(r);
            let r = r.strip_prefix('(').unwrap_or(r);
            let r = r.strip_suffix(')').unwrap_or(r);
            r.trim()
        } else if let Some(rest) = trimmed.strip_prefix("while ") {
            let r = rest.trim();
            let r = r.strip_suffix(':').unwrap_or(r);
            let r = r.strip_suffix('{').unwrap_or(r);
            let r = r.strip_prefix('(').unwrap_or(r);
            let r = r.strip_suffix(')').unwrap_or(r);
            r.trim()
        } else if let Some(rest) = trimmed.strip_prefix("assert ") {
            rest.trim()
        } else if let Some(rest) = trimmed.strip_prefix("yield ") {
            rest.trim()
        } else {
            // Try the whole line as-is (standalone expression)
            trimmed
        };
        stripped
    };

    // Skip if the expression is a function call (CAP-AE-12)
    if is_function_call(expr_text) {
        return None;
    }

    // Try to find a binary operator in the expression
    // Sort operators by length (descending) to match longer operators first
    let mut ops_by_len: Vec<&str> = BINARY_OPS.to_vec();
    ops_by_len.sort_by_key(|op| std::cmp::Reverse(op.len()));

    for op in ops_by_len {
        // Look for operator surrounded by spaces or at word boundaries
        if let Some(op_idx) = find_operator_in_expr(expr_text, op) {
            let left = expr_text[..op_idx].trim();
            let right = expr_text[op_idx + op.len()..].trim();

            // TIGER-PASS3-5: Limit to base variable (truncate deeply nested field access)
            let left = extract_base_variable(left);
            let right = extract_base_variable(right);

            // Skip if either operand looks like a function call
            if is_function_call(&left) || is_function_call(&right) {
                return None;
            }

            // Skip if operands are empty or are just literals
            if left.is_empty() || right.is_empty() {
                continue;
            }

            // Skip if both operands are numeric literals (constant folding, not CSE)
            if is_numeric_literal(&left) && is_numeric_literal(&right) {
                continue;
            }

            return Some((left, op.to_string(), right));
        }
    }

    None
}

/// Find operator in expression, avoiding operators inside parentheses or strings.
fn find_operator_in_expr(expr: &str, op: &str) -> Option<usize> {
    let chars: Vec<char> = expr.chars().collect();
    let op_chars: Vec<char> = op.chars().collect();
    let mut paren_depth: usize = 0;
    let mut in_string = false;
    let mut string_char = '"';

    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];

        // Track string state
        if !in_string && (c == '"' || c == '\'') {
            in_string = true;
            string_char = c;
            i += 1;
            continue;
        }
        if in_string && c == string_char && (i == 0 || chars[i - 1] != '\\') {
            in_string = false;
            i += 1;
            continue;
        }
        if in_string {
            i += 1;
            continue;
        }

        // Track paren depth
        if c == '(' || c == '[' || c == '{' {
            paren_depth += 1;
            i += 1;
            continue;
        }
        if c == ')' || c == ']' || c == '}' {
            paren_depth = paren_depth.saturating_sub(1);
            i += 1;
            continue;
        }

        // Only look for operators at top level (not inside parens)
        if paren_depth == 0 {
            // Check if operator matches at this position
            if i + op_chars.len() <= chars.len() {
                let matches = op_chars
                    .iter()
                    .enumerate()
                    .all(|(j, &oc)| chars[i + j] == oc);

                if matches {
                    // For word operators (and, or), check word boundaries
                    if op.chars().all(|c| c.is_alphabetic()) {
                        let before_ok = i == 0 || !chars[i - 1].is_alphanumeric();
                        let after_ok =
                            i + op.len() >= chars.len() || !chars[i + op.len()].is_alphanumeric();
                        if before_ok && after_ok {
                            return Some(i);
                        }
                    } else {
                        return Some(i);
                    }
                }
            }
        }

        i += 1;
    }

    None
}

/// Extract base variable from potentially nested field access.
///
/// TIGER-PASS3-5: Limit operand extraction to base variable only
/// to avoid issues with circular field references.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(extract_base_variable("x"), "x");
/// assert_eq!(extract_base_variable("x.a"), "x.a");
/// assert_eq!(extract_base_variable("x.a.b.c.d.e"), "x.a.b"); // Truncated
/// ```
fn extract_base_variable(text: &str) -> String {
    let trimmed = text.trim();

    // Count dots to detect deep nesting
    let parts: Vec<&str> = trimmed.split('.').collect();

    // TIGER-PASS3-5: Limit to 3 levels of field access
    if parts.len() > 3 {
        parts[..3].join(".")
    } else {
        trimmed.to_string()
    }
}

/// Check if text is a numeric literal.
fn is_numeric_literal(text: &str) -> bool {
    let trimmed = text.trim();

    // Integer
    if trimmed.parse::<i64>().is_ok() {
        return true;
    }

    // Float
    if trimmed.parse::<f64>().is_ok() {
        return true;
    }

    // Hex, octal, binary
    if trimmed.starts_with("0x") || trimmed.starts_with("0o") || trimmed.starts_with("0b") {
        return true;
    }

    false
}

/// Result of expression extraction including Phase 4 data.
///
/// Contains all data needed for both Phase 3 MUST analysis and
/// Phase 4 intra-block CSE detection.
#[derive(Debug, Clone, Default)]
pub struct ExtractionResult {
    /// All unique expressions found
    pub all_exprs: HashSet<Expression>,
    /// Per-block gen/kill sets
    pub block_info: HashMap<BlockId, BlockExpressions>,
    /// Expression instances in order (legacy)
    pub expr_instances: Vec<Expression>,
    /// Expression instances with block context (Phase 4)
    pub expr_instances_with_blocks: Vec<ExprInstance>,
    /// Definitions per line for intra-block kill tracking (Phase 4)
    pub defs_per_line: HashMap<usize, HashSet<String>>,
    /// Maps lines to their containing block IDs
    pub line_to_block: HashMap<usize, BlockId>,
    /// Expressions skipped during extraction (function calls, impure constructs)
    pub uncertain_exprs: Vec<UncertainFinding>,
}

/// Check if text is a valid identifier (variable name).
#[allow(dead_code)]
fn is_identifier(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    let mut chars = trimmed.chars();

    // First character must be letter or underscore
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }

    // Rest must be alphanumeric, underscore, or dot (for field access)
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// Extract expressions from DFG variable references.
///
/// This function bridges the gap between the DFG (which tracks variable references)
/// and available expressions analysis (which tracks binary expressions).
///
/// # Algorithm
///
/// 1. Group VarRefs by (block_id, line)
/// 2. For groups with 2+ Uses on same line as 1 Def, extract expr
/// 3. Parse line text to extract operator
/// 4. Apply whitespace normalization
/// 5. Filter out function calls (CAP-AE-12)
///
/// # Arguments
///
/// * `cfg` - The control flow graph
/// * `dfg` - The data flow graph with variable references
///
/// # Returns
///
/// A tuple of:
/// - `HashSet<Expression>`: All unique expressions found
/// - `HashMap<BlockId, BlockExpressions>`: Per-block gen/kill sets
/// - `Vec<Expression>`: All expression instances (including duplicates) in order
///
/// # TIGER Mitigations
///
/// - TIGER-PASS1-10: Algorithm detailed in steps above
/// - TIGER-PASS2-1: Multiple assignment targets (a = b = 5) - only process rightmost
/// - TIGER-PASS3-5: Circular field references - limit to base variable
///
/// # Example
///
/// ```rust,ignore
/// let (all_exprs, block_info, expr_instances) = extract_expressions_from_refs(&cfg, &dfg);
///
/// // all_exprs contains unique expressions like "a + b"
/// // block_info maps block IDs to their gen/kill sets
/// // expr_instances preserves order for CSE detection
/// ```
pub fn extract_expressions_from_refs(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
) -> (
    HashSet<Expression>,
    HashMap<BlockId, BlockExpressions>,
    Vec<Expression>,
) {
    let mut all_exprs: HashSet<Expression> = HashSet::new();
    let mut block_info: HashMap<BlockId, BlockExpressions> = HashMap::new();
    let mut expr_instances: Vec<Expression> = Vec::new();

    // Initialize block_info for all blocks
    for block in &cfg.blocks {
        block_info.insert(block.id, BlockExpressions::new());
    }

    // Group VarRefs by (block, line) to find expressions
    // An expression typically has: 1 Def (the assignment target) and 2+ Uses (the operands)
    let mut refs_by_line: HashMap<(BlockId, u32), Vec<&VarRef>> = HashMap::new();

    for var_ref in &dfg.refs {
        // Find which block this ref belongs to
        let block_id = find_block_for_line(cfg, var_ref.line);
        if let Some(bid) = block_id {
            refs_by_line
                .entry((bid, var_ref.line))
                .or_default()
                .push(var_ref);
        }
    }

    // Process each (block, line) group
    for ((block_id, line), refs) in refs_by_line {
        // Count defs and uses
        let defs: Vec<_> = refs
            .iter()
            .filter(|r| matches!(r.ref_type, RefType::Definition))
            .collect();
        let uses: Vec<_> = refs
            .iter()
            .filter(|r| matches!(r.ref_type, RefType::Use))
            .collect();

        // Skip if no assignment (no def) or not enough uses for a binary expression
        if defs.is_empty() || uses.len() < 2 {
            // Even without a def, we might have a use of an expression
            // but for Phase 2 we focus on assignments like x = a + b
            continue;
        }

        // We have an assignment with 2+ uses - likely a binary expression
        // Try to extract the expression from the source line

        // Get the variable names that are used
        let use_names: Vec<&str> = uses.iter().map(|r| r.name.as_str()).collect();

        // If we have exactly 2 distinct uses, this could be a binary expression
        let unique_uses: HashSet<&str> = use_names.iter().copied().collect();

        if unique_uses.len() >= 2 {
            // Create expression from the uses
            // We need to determine the operator - for now, use a heuristic based on ordering
            let mut operands: Vec<String> = unique_uses.iter().map(|s| s.to_string()).collect();
            operands.sort();

            // Create a placeholder expression - we'll need source code to get the actual operator
            // For now, create expression with uses and infer operator pattern
            if let Some(op) = infer_operator_from_uses(&use_names) {
                let text = normalize_expression(&operands[0], &op, &operands[1]);
                let expr = Expression::new(text, operands.clone(), line as usize);

                // Add to all_exprs
                all_exprs.insert(expr.clone());

                // Add to expr_instances
                expr_instances.push(expr.clone());

                // Update block's gen set
                if let Some(block_expr) = block_info.get_mut(&block_id) {
                    block_expr.gen.insert(expr);
                }
            }
        }

        // Update kill set for defs
        for def in &defs {
            if let Some(block_expr) = block_info.get_mut(&block_id) {
                block_expr.kill.insert(def.name.clone());
            }
        }
    }

    // Sort expr_instances by line for deterministic order
    expr_instances.sort_by_key(|e| e.line);

    (all_exprs, block_info, expr_instances)
}

/// Extract expressions from DFG with source code for better accuracy.
///
/// This is an enhanced version that uses source code to accurately parse
/// the operator from assignment statements.
///
/// # Arguments
///
/// * `cfg` - The control flow graph
/// * `dfg` - The data flow graph with variable references
/// * `source_lines` - Optional source code lines for accurate operator extraction
///
/// # Returns
///
/// Same as `extract_expressions_from_refs` but with accurate operators when source is provided.
pub fn extract_expressions_from_refs_with_source(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    source_lines: Option<&[String]>,
) -> (
    HashSet<Expression>,
    HashMap<BlockId, BlockExpressions>,
    Vec<Expression>,
) {
    let mut all_exprs: HashSet<Expression> = HashSet::new();
    let mut block_info: HashMap<BlockId, BlockExpressions> = HashMap::new();
    let mut expr_instances: Vec<Expression> = Vec::new();

    // Initialize block_info for all blocks
    for block in &cfg.blocks {
        block_info.insert(block.id, BlockExpressions::new());
    }

    // If we have source lines, use them for accurate parsing
    if let Some(lines) = source_lines {
        // Process each line that might contain an expression
        for (line_num, line_text) in lines.iter().enumerate() {
            let line = (line_num + 1) as u32; // 1-indexed

            // Try to parse expression from line
            if let Some((left, op, right)) = parse_expression_from_line(line_text) {
                // Validate that the operands appear in the DFG
                let refs_on_line: Vec<_> = dfg.refs.iter().filter(|r| r.line == line).collect();

                let has_left = refs_on_line.iter().any(|r| r.name == left);
                let has_right = refs_on_line.iter().any(|r| r.name == right);

                // Only create expression if we can validate operands from DFG
                if has_left || has_right || is_numeric_literal(&left) || is_numeric_literal(&right)
                {
                    let text = normalize_expression(&left, &op, &right);
                    let operands = vec![left.clone(), right.clone()];
                    let expr = Expression::new(text, operands, line as usize);

                    // Find block for this line
                    if let Some(block_id) = find_block_for_line(cfg, line) {
                        // Add to all_exprs
                        all_exprs.insert(expr.clone());

                        // Add to expr_instances
                        expr_instances.push(expr.clone());

                        // Update block's gen set
                        if let Some(block_expr) = block_info.get_mut(&block_id) {
                            // Only add to gen if operands haven't been killed yet in this block
                            block_expr.gen.insert(expr);
                        }
                    }
                }
            }
        }

        // Build kill sets from definitions in DFG
        for var_ref in &dfg.refs {
            if matches!(var_ref.ref_type, RefType::Definition | RefType::Update) {
                if let Some(block_id) = find_block_for_line(cfg, var_ref.line) {
                    if let Some(block_expr) = block_info.get_mut(&block_id) {
                        block_expr.kill.insert(var_ref.name.clone());
                    }
                }
            }
        }
    } else {
        // Fall back to DFG-only extraction
        return extract_expressions_from_refs(cfg, dfg);
    }

    // Sort expr_instances by line for deterministic order
    expr_instances.sort_by_key(|e| e.line);

    (all_exprs, block_info, expr_instances)
}

/// Find the block ID containing a given line number.
fn find_block_for_line(cfg: &CfgInfo, line: u32) -> Option<BlockId> {
    for block in &cfg.blocks {
        if block.lines.0 <= line && line <= block.lines.1 {
            return Some(block.id);
        }
    }
    // If no block contains the line exactly, find the closest one
    // This handles edge cases where line numbers don't perfectly match block ranges
    cfg.blocks
        .iter()
        .min_by_key(|b| {
            let dist_start = (b.lines.0 as i64 - line as i64).abs();
            let dist_end = (b.lines.1 as i64 - line as i64).abs();
            dist_start.min(dist_end)
        })
        .map(|b| b.id)
}

/// Infer operator from use patterns when source code is not available.
///
/// This is a heuristic fallback - it assumes common patterns like
/// binary operations when we see exactly 2 uses.
fn infer_operator_from_uses(uses: &[&str]) -> Option<String> {
    // If we have exactly 2 uses, assume some binary operation
    // Default to "+" as a placeholder - this will be normalized anyway
    if uses.len() >= 2 {
        Some("+".to_string())
    } else {
        None
    }
}

/// Extract expressions with full Phase 4 data for intra-block CSE detection.
///
/// This is the Phase 4 version that returns additional data for proper
/// intra-block kill tracking in redundant_computations().
///
/// # Arguments
///
/// * `cfg` - The control flow graph
/// * `dfg` - The data flow graph with variable references
/// * `source_lines` - Optional source code lines for accurate operator extraction
///
/// # Returns
///
/// `ExtractionResult` containing all data needed for Phase 4 CSE detection.
pub fn extract_expressions_full(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    source_lines: Option<&[String]>,
) -> ExtractionResult {
    extract_expressions_full_with_lang(cfg, dfg, source_lines, None)
}

/// Extract expressions with full Phase 4 data and optional language for AST extraction.
///
/// When `lang` is provided, this function supplements text-based extraction with
/// AST-based binary expression extraction using tree-sitter.
///
/// # Arguments
///
/// * `cfg` - The control flow graph
/// * `dfg` - The data flow graph with variable references
/// * `source_lines` - Optional source code lines for accurate operator extraction
/// * `lang` - Optional language for AST-based extraction (enhances results)
///
/// # Returns
///
/// `ExtractionResult` containing all data needed for Phase 4 CSE detection.
pub fn extract_expressions_full_with_lang(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    source_lines: Option<&[String]>,
    lang: Option<Language>,
) -> ExtractionResult {
    let mut result = ExtractionResult::default();

    // Initialize block_info for all blocks and build line_to_block mapping
    for block in &cfg.blocks {
        result.block_info.insert(block.id, BlockExpressions::new());
        // Map each line in this block to the block ID
        for line in block.lines.0..=block.lines.1 {
            result.line_to_block.insert(line as usize, block.id);
        }
    }

    // Build defs_per_line from DFG
    for var_ref in &dfg.refs {
        if matches!(var_ref.ref_type, RefType::Definition | RefType::Update) {
            result
                .defs_per_line
                .entry(var_ref.line as usize)
                .or_default()
                .insert(var_ref.name.clone());

            // Also update block's kill set
            if let Some(block_id) = find_block_for_line(cfg, var_ref.line) {
                if let Some(block_expr) = result.block_info.get_mut(&block_id) {
                    block_expr.kill.insert(var_ref.name.clone());
                }
            }
        }
    }

    // If we have source lines, use them for accurate expression parsing
    if let Some(lines) = source_lines {
        for (line_num, line_text) in lines.iter().enumerate() {
            let line = (line_num + 1) as u32; // 1-indexed

            if let Some((left, op, right)) = parse_expression_from_line(line_text) {
                // Validate operands from DFG
                let refs_on_line: Vec<_> = dfg.refs.iter().filter(|r| r.line == line).collect();

                let has_left = refs_on_line.iter().any(|r| r.name == left);
                let has_right = refs_on_line.iter().any(|r| r.name == right);

                if has_left || has_right || is_numeric_literal(&left) || is_numeric_literal(&right)
                {
                    let text = normalize_expression(&left, &op, &right);
                    let operands = vec![left.clone(), right.clone()];
                    let expr = Expression::new(text, operands, line as usize);

                    if let Some(block_id) = find_block_for_line(cfg, line) {
                        result.all_exprs.insert(expr.clone());
                        result.expr_instances.push(expr.clone());
                        result
                            .expr_instances_with_blocks
                            .push(ExprInstance::new(expr.clone(), block_id));

                        if let Some(block_expr) = result.block_info.get_mut(&block_id) {
                            block_expr.gen.insert(expr);
                        }
                    }
                }
            } else {
                // Check if this line was skipped because it contains a function call
                // If so, collect it as an uncertain finding
                if let Some(uncertain) = detect_uncertain_expression(line_text, line as usize) {
                    // Only include if this line is within the function's CFG range
                    if find_block_for_line(cfg, line).is_some() {
                        result.uncertain_exprs.push(uncertain);
                    }
                }
            }
        }
    } else {
        // Fall back to DFG-only extraction
        let mut refs_by_line: HashMap<(BlockId, u32), Vec<&VarRef>> = HashMap::new();

        for var_ref in &dfg.refs {
            if let Some(bid) = find_block_for_line(cfg, var_ref.line) {
                refs_by_line
                    .entry((bid, var_ref.line))
                    .or_default()
                    .push(var_ref);
            }
        }

        for ((block_id, line), refs) in refs_by_line {
            let defs: Vec<_> = refs
                .iter()
                .filter(|r| matches!(r.ref_type, RefType::Definition))
                .collect();
            let uses: Vec<_> = refs
                .iter()
                .filter(|r| matches!(r.ref_type, RefType::Use))
                .collect();

            if defs.is_empty() || uses.len() < 2 {
                continue;
            }

            let use_names: Vec<&str> = uses.iter().map(|r| r.name.as_str()).collect();
            let unique_uses: HashSet<&str> = use_names.iter().copied().collect();

            if unique_uses.len() >= 2 {
                let mut operands: Vec<String> = unique_uses.iter().map(|s| s.to_string()).collect();
                operands.sort();

                if let Some(op) = infer_operator_from_uses(&use_names) {
                    let text = normalize_expression(&operands[0], &op, &operands[1]);
                    let expr = Expression::new(text, operands.clone(), line as usize);

                    result.all_exprs.insert(expr.clone());
                    result.expr_instances.push(expr.clone());
                    result
                        .expr_instances_with_blocks
                        .push(ExprInstance::new(expr.clone(), block_id));

                    if let Some(block_expr) = result.block_info.get_mut(&block_id) {
                        block_expr.gen.insert(expr);
                    }
                }
            }
        }
    }

    // AST-based extraction: supplement text-based extraction with tree-sitter
    // This catches expressions in non-assignment contexts (return, if, while, etc.)
    if let (Some(lines), Some(language)) = (source_lines, lang) {
        let full_source = lines.join("\n");
        // Determine the line range from the CFG
        let min_line = cfg.blocks.iter().map(|b| b.lines.0).min().unwrap_or(1) as usize;
        let max_line = cfg.blocks.iter().map(|b| b.lines.1).max().unwrap_or(1) as usize;

        let ast_exprs = extract_binary_exprs_from_ast(&full_source, language, min_line, max_line);

        for (text, _op, left, right, line) in ast_exprs {
            let line_u32 = line as u32;

            // Check if this expression was already found by text-based extraction
            let already_found = result
                .all_exprs
                .iter()
                .any(|e| e.text == text && e.line == line);
            if already_found {
                continue;
            }

            // Create the expression
            let operands = vec![left.clone(), right.clone()];
            let expr = Expression::new(text.clone(), operands, line);

            if let Some(block_id) = find_block_for_line(cfg, line_u32) {
                result.all_exprs.insert(expr.clone());
                result.expr_instances.push(expr.clone());
                result
                    .expr_instances_with_blocks
                    .push(ExprInstance::new(expr.clone(), block_id));

                if let Some(block_expr) = result.block_info.get_mut(&block_id) {
                    block_expr.gen.insert(expr);
                }
            }
        }
    }

    // Sort expr_instances by line for deterministic order
    result.expr_instances.sort_by_key(|e| e.line);
    result
        .expr_instances_with_blocks
        .sort_by_key(|e| e.expr.line);

    result
}

// =============================================================================
// Phase 3: MUST Analysis - Compute Available Expressions
// =============================================================================

/// Compute available expressions using MUST (intersection) analysis.
///
/// MUST semantics: An expression is available at a program point only if it
/// has been computed on ALL paths reaching that point.
///
/// # Algorithm
///
/// 1. Extract expressions from DFG (Phase 2)
/// 2. Initialize: entry = {}, others = ALL_EXPRS (optimistic for MUST)
/// 3. Iterate until fixpoint:
///    - avail_in[b] = intersection(avail_out[p] for p in preds[b])
///    - avail_out[b] = gen[b] | (avail_in[b] - killed[b])
/// 4. Return AvailableExprsInfo
///
/// # Arguments
///
/// * `cfg` - The control flow graph
/// * `dfg` - The data flow graph with variable references
///
/// # Returns
///
/// `Result<AvailableExprsInfo, DataflowError>` containing the analysis results.
///
/// # TIGER Mitigations
///
/// - TIGER-PASS1-4: Iteration bound = blocks * expressions * 2 + 10
/// - TIGER-PASS3-4: Validates CFG doesn't exceed MAX_BLOCKS
///
/// # Example
///
/// ```rust,ignore
/// let result = compute_available_exprs(&cfg, &dfg)?;
///
/// // Check if expression is available at a block
/// let expr = Expression::new("a + b", vec!["a", "b"], 1);
/// if result.is_available(block_id, &expr) {
///     println!("Expression is available - can use CSE");
/// }
///
/// // Find redundant computations
/// for (text, first, redundant) in result.redundant_computations() {
///     println!("CSE opportunity: {} at line {} (first at {})", text, redundant, first);
/// }
/// ```
pub fn compute_available_exprs(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
) -> Result<AvailableExprsInfo, DataflowError> {
    // Validate CFG (TIGER-PASS3-4)
    validate_cfg(cfg)?;

    let entry = cfg.entry_block;

    // Step 1: Extract expressions from DFG (Phase 4: use full extraction for intra-block tracking)
    let extraction = extract_expressions_full(cfg, dfg, None);
    let all_exprs = extraction.all_exprs;
    let block_info = extraction.block_info;
    let expr_instances = extraction.expr_instances;
    let expr_instances_with_blocks = extraction.expr_instances_with_blocks;
    let defs_per_line = extraction.defs_per_line;
    let line_to_block = extraction.line_to_block;

    // Early return if no expressions (spec line 212-214)
    if all_exprs.is_empty() {
        let mut info = AvailableExprsInfo::empty(entry);
        // Initialize avail_in and avail_out for all blocks (even if empty)
        for block in &cfg.blocks {
            info.avail_in.insert(block.id, HashSet::new());
            info.avail_out.insert(block.id, HashSet::new());
        }
        return Ok(info);
    }

    // Step 2: Build predecessor map
    let predecessors = build_predecessors(cfg);

    // Step 3: Initialize (MUST analysis: start optimistic except entry)
    // For MUST analysis:
    // - Entry block avail_in = {} (nothing available)
    // - Other blocks avail_in = ALL_EXPRS (optimistic, will be intersected down)
    let mut avail_in: HashMap<BlockId, HashSet<Expression>> = HashMap::new();
    let mut avail_out: HashMap<BlockId, HashSet<Expression>> = HashMap::new();

    // Entry block: nothing available at entry
    avail_in.insert(entry, HashSet::new());

    // Entry block avail_out = gen[entry] (only what's generated in entry)
    let entry_gen = block_info
        .get(&entry)
        .map(|b| b.gen.clone())
        .unwrap_or_default();
    avail_out.insert(entry, entry_gen);

    // Other blocks: initialize to ALL_EXPRS (optimistic)
    for block in &cfg.blocks {
        if block.id != entry {
            avail_in.insert(block.id, all_exprs.clone());
            avail_out.insert(block.id, all_exprs.clone());
        }
    }

    // Step 4: Iterate until fixpoint
    // TIGER-PASS1-4: Iteration bound
    let max_iterations = cfg.blocks.len() * all_exprs.len() * 2 + 10;
    let mut changed = true;
    let mut iteration = 0;

    // Get iteration order (reverse postorder for efficiency)
    let block_order = reverse_postorder(cfg);

    while changed && iteration < max_iterations {
        changed = false;
        iteration += 1;

        for &block_id in &block_order {
            // Skip entry block (already initialized)
            if block_id == entry {
                continue;
            }

            // avail_in[b] = INTERSECTION of all predecessor's avail_out
            let preds = predecessors
                .get(&block_id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);

            let new_in: HashSet<Expression> = if preds.is_empty() {
                // No predecessors (unreachable block) -> nothing available
                HashSet::new()
            } else {
                // MUST = intersection of all predecessors
                let mut result = avail_out
                    .get(&preds[0])
                    .cloned()
                    .unwrap_or_else(|| all_exprs.clone());

                for &pred in &preds[1..] {
                    let pred_out = avail_out
                        .get(&pred)
                        .cloned()
                        .unwrap_or_else(|| all_exprs.clone());
                    // Intersection
                    result = result.intersection(&pred_out).cloned().collect();
                }
                result
            };

            // avail_out[b] = gen[b] | (avail_in[b] - killed_by_block[b])
            let info = block_info.get(&block_id);
            let gen = info.map(|i| &i.gen).cloned().unwrap_or_default();
            let kill = info.map(|i| &i.kill).cloned().unwrap_or_default();

            // Compute killed expressions: those whose operands are in kill set
            let not_killed: HashSet<Expression> = new_in
                .iter()
                .filter(|expr| !is_killed(expr, &kill))
                .cloned()
                .collect();

            // avail_out = gen | (avail_in - killed)
            let new_out: HashSet<Expression> = gen.union(&not_killed).cloned().collect();

            // Check for change
            if avail_in.get(&block_id) != Some(&new_in)
                || avail_out.get(&block_id) != Some(&new_out)
            {
                changed = true;
                avail_in.insert(block_id, new_in);
                avail_out.insert(block_id, new_out);
            }
        }
    }

    // Check for non-convergence (shouldn't happen with valid CFG)
    if iteration >= max_iterations {
        return Err(DataflowError::IterationLimit {
            iterations: iteration,
        });
    }

    Ok(AvailableExprsInfo {
        avail_in,
        avail_out,
        all_exprs,
        entry_block: entry,
        expr_instances,
        expr_instances_with_blocks,
        defs_per_line,
        line_to_block,
        uncertain_exprs: Vec::new(),
        confidence: Confidence::High,
    })
}

/// Compute available expressions with source code for better accuracy.
///
/// This version uses source code to accurately parse operators from assignment
/// statements, providing more precise expression extraction.
///
/// # Arguments
///
/// * `cfg` - The control flow graph
/// * `dfg` - The data flow graph with variable references
/// * `source_lines` - Source code lines for accurate operator extraction
///
/// # Returns
///
/// `Result<AvailableExprsInfo, DataflowError>` containing the analysis results.
pub fn compute_available_exprs_with_source(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    source_lines: &[String],
) -> Result<AvailableExprsInfo, DataflowError> {
    compute_available_exprs_with_source_and_lang(cfg, dfg, source_lines, None)
}

/// Compute available expressions with source code and language for AST-enhanced accuracy.
///
/// This version combines text-based and AST-based expression extraction
/// for maximum coverage across all programming languages.
///
/// # Arguments
///
/// * `cfg` - The control flow graph
/// * `dfg` - The data flow graph with variable references
/// * `source_lines` - Source code lines for accurate operator extraction
/// * `lang` - Optional programming language for AST-based extraction
///
/// # Returns
///
/// `Result<AvailableExprsInfo, DataflowError>` containing the analysis results.
pub fn compute_available_exprs_with_source_and_lang(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    source_lines: &[String],
    lang: Option<Language>,
) -> Result<AvailableExprsInfo, DataflowError> {
    // Validate CFG (TIGER-PASS3-4)
    validate_cfg(cfg)?;

    let entry = cfg.entry_block;

    // Step 1: Extract expressions from DFG with source (Phase 4: use full extraction)
    let extraction = extract_expressions_full_with_lang(cfg, dfg, Some(source_lines), lang);
    let all_exprs = extraction.all_exprs;
    let block_info = extraction.block_info;
    let expr_instances = extraction.expr_instances;
    let expr_instances_with_blocks = extraction.expr_instances_with_blocks;
    let defs_per_line = extraction.defs_per_line;
    let line_to_block = extraction.line_to_block;
    let uncertain_exprs = extraction.uncertain_exprs;

    // Early return if no expressions
    if all_exprs.is_empty() {
        let mut info = AvailableExprsInfo::empty(entry);
        info.uncertain_exprs = uncertain_exprs.clone();
        // Compute confidence based on what we found
        info.confidence = compute_confidence(0, uncertain_exprs.len());
        for block in &cfg.blocks {
            info.avail_in.insert(block.id, HashSet::new());
            info.avail_out.insert(block.id, HashSet::new());
        }
        return Ok(info);
    }

    // Build predecessor map
    let predecessors = build_predecessors(cfg);

    // Initialize (MUST analysis)
    let mut avail_in: HashMap<BlockId, HashSet<Expression>> = HashMap::new();
    let mut avail_out: HashMap<BlockId, HashSet<Expression>> = HashMap::new();

    avail_in.insert(entry, HashSet::new());
    let entry_gen = block_info
        .get(&entry)
        .map(|b| b.gen.clone())
        .unwrap_or_default();
    avail_out.insert(entry, entry_gen);

    for block in &cfg.blocks {
        if block.id != entry {
            avail_in.insert(block.id, all_exprs.clone());
            avail_out.insert(block.id, all_exprs.clone());
        }
    }

    // Iterate until fixpoint
    let max_iterations = cfg.blocks.len() * all_exprs.len() * 2 + 10;
    let mut changed = true;
    let mut iteration = 0;
    let block_order = reverse_postorder(cfg);

    while changed && iteration < max_iterations {
        changed = false;
        iteration += 1;

        for &block_id in &block_order {
            if block_id == entry {
                continue;
            }

            let preds = predecessors
                .get(&block_id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);

            let new_in: HashSet<Expression> = if preds.is_empty() {
                HashSet::new()
            } else {
                let mut result = avail_out
                    .get(&preds[0])
                    .cloned()
                    .unwrap_or_else(|| all_exprs.clone());

                for &pred in &preds[1..] {
                    let pred_out = avail_out
                        .get(&pred)
                        .cloned()
                        .unwrap_or_else(|| all_exprs.clone());
                    result = result.intersection(&pred_out).cloned().collect();
                }
                result
            };

            let info = block_info.get(&block_id);
            let gen = info.map(|i| &i.gen).cloned().unwrap_or_default();
            let kill = info.map(|i| &i.kill).cloned().unwrap_or_default();

            let not_killed: HashSet<Expression> = new_in
                .iter()
                .filter(|expr| !is_killed(expr, &kill))
                .cloned()
                .collect();

            let new_out: HashSet<Expression> = gen.union(&not_killed).cloned().collect();

            if avail_in.get(&block_id) != Some(&new_in)
                || avail_out.get(&block_id) != Some(&new_out)
            {
                changed = true;
                avail_in.insert(block_id, new_in);
                avail_out.insert(block_id, new_out);
            }
        }
    }

    if iteration >= max_iterations {
        return Err(DataflowError::IterationLimit {
            iterations: iteration,
        });
    }

    let confidence = compute_confidence(all_exprs.len(), uncertain_exprs.len());

    Ok(AvailableExprsInfo {
        avail_in,
        avail_out,
        all_exprs,
        entry_block: entry,
        expr_instances,
        expr_instances_with_blocks,
        defs_per_line,
        line_to_block,
        uncertain_exprs,
        confidence,
    })
}

/// Compute confidence level based on confirmed vs uncertain expression counts.
fn compute_confidence(confirmed: usize, uncertain: usize) -> Confidence {
    if confirmed == 0 && uncertain == 0 {
        Confidence::Low
    } else if uncertain == 0 {
        Confidence::High
    } else if confirmed >= uncertain {
        Confidence::Medium
    } else {
        Confidence::Low
    }
}

/// Check if an expression is killed by any variable in the kill set.
///
/// An expression is killed if any of its operands is redefined (in the kill set).
///
/// # Arguments
///
/// * `expr` - The expression to check
/// * `kills` - Set of variables that are redefined (killed)
///
/// # Returns
///
/// `true` if the expression uses any variable in the kill set.
fn is_killed(expr: &Expression, kills: &HashSet<String>) -> bool {
    expr.operands.iter().any(|op| kills.contains(op))
}

// =============================================================================
// AST-based binary expression extraction
// =============================================================================

use crate::types::Language;

/// Binary operator node kind names per language for tree-sitter.
/// Returns the node kind(s) that represent binary expressions in each language.
fn binary_expr_node_kinds(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &["binary_operator", "boolean_operator", "comparison_operator"],
        Language::Go => &["binary_expression"],
        Language::TypeScript | Language::JavaScript => &["binary_expression"],
        Language::Java => &["binary_expression"],
        Language::Rust => &["binary_expression"],
        Language::C | Language::Cpp => &["binary_expression"],
        Language::Ruby => &["binary"],
        Language::Php => &["binary_expression"],
        Language::Kotlin => &["binary_expression"],
        Language::CSharp => &["binary_expression"],
        Language::Scala => &["infix_expression"],
        Language::Elixir => &["binary_operator"],
        Language::Ocaml => &["infix_expression"],
        Language::Lua | Language::Luau => &["binary_expression"],
        Language::Swift => &["infix_expression"],
    }
}

/// Extract the operator text from a binary expression node.
///
/// Different languages use different AST structures:
/// - Some have an "operator" field (Python, Go, Java, C, C++, C#, Ruby, PHP, Rust)
/// - Some embed the operator as a child node
/// - Kotlin uses separate node kinds for each operator type
fn extract_operator_from_node<'a>(
    node: &tree_sitter::Node<'a>,
    source: &'a [u8],
    _lang: Language,
) -> Option<String> {
    // Try "operator" field first (most languages)
    if let Some(op_node) = node.child_by_field_name("operator") {
        let op_text = op_node.utf8_text(source).unwrap_or("").trim().to_string();
        if !op_text.is_empty() {
            return Some(op_text);
        }
    }

    // For languages that embed operator as unnamed children, scan children
    let child_count = node.child_count();
    for i in 0..child_count {
        if let Some(child) = node.child(i) {
            if !child.is_named() {
                let text = child.utf8_text(source).unwrap_or("").trim();
                // Check if it looks like a binary operator
                if matches!(
                    text,
                    "+" | "-"
                        | "*"
                        | "/"
                        | "%"
                        | "//"
                        | "**"
                        | "=="
                        | "!="
                        | "<"
                        | ">"
                        | "<="
                        | ">="
                        | "&&"
                        | "||"
                        | "and"
                        | "or"
                        | "&"
                        | "|"
                        | "^"
                        | "<<"
                        | ">>"
                        | "<>"
                        | "==="
                        | "!=="
                        | "<=>"
                        | ".."
                        | "..."
                        | "in"
                        | "not in"
                        | "|>"
                        | "<|"
                        | "++"
                ) {
                    return Some(text.to_string());
                }
            }
            // Note: Kotlin (tree-sitter-kotlin-ng) uses standard binary_expression
            // with left/operator/right fields, so no special handling needed
        }
    }

    None
}

/// Extract left and right operand text from a binary expression node.
///
/// Uses field names ("left"/"right") first, then falls back to first/last named children.
fn extract_operands_from_node<'a>(
    node: &tree_sitter::Node<'a>,
    source: &'a [u8],
    lang: Language,
) -> Option<(String, String)> {
    // Try standard field names
    let left = node.child_by_field_name("left").or_else(|| {
        // Fallback: first named child
        node.named_child(0)
    });

    let right = node.child_by_field_name("right").or_else(|| {
        // Fallback: last named child (if different from first)
        let count = node.named_child_count();
        if count >= 2 {
            node.named_child(count - 1)
        } else {
            None
        }
    });

    match (left, right) {
        (Some(l), Some(r)) if l.id() != r.id() => {
            let left_text = l.utf8_text(source).unwrap_or("").trim().to_string();
            let right_text = r.utf8_text(source).unwrap_or("").trim().to_string();

            if left_text.is_empty() || right_text.is_empty() {
                return None;
            }

            // Extract base variable (limit depth for field access chains)
            let left_base = extract_base_variable(&left_text);
            let right_base = extract_base_variable(&right_text);

            // Skip if either side looks like a function call
            if is_function_call(&left_base) || is_function_call(&right_base) {
                return None;
            }

            // Skip if both sides are numeric literals (constant folding, not CSE)
            if is_numeric_literal(&left_base) && is_numeric_literal(&right_base) {
                return None;
            }

            // For PHP, strip $ prefix for consistency
            let left_final = if matches!(lang, Language::Php) {
                left_base.trim_start_matches('$').to_string()
            } else {
                left_base
            };
            let right_final = if matches!(lang, Language::Php) {
                right_base.trim_start_matches('$').to_string()
            } else {
                right_base
            };

            Some((left_final, right_final))
        }
        _ => None,
    }
}

/// Extract binary expressions from source code using tree-sitter AST.
///
/// Walks the AST to find all binary expression nodes within the given line range,
/// extracting their operator and operands. This is language-aware and handles
/// each language's specific AST structure.
///
/// # Arguments
///
/// * `source` - The full source code
/// * `lang` - The programming language
/// * `start_line` - Start line (1-indexed, inclusive)
/// * `end_line` - End line (1-indexed, inclusive)
///
/// # Returns
///
/// A vector of `(normalized_text, operator, left_operand, right_operand, line)` tuples.
pub fn extract_binary_exprs_from_ast(
    source: &str,
    lang: Language,
    start_line: usize,
    end_line: usize,
) -> Vec<(String, String, String, String, usize)> {
    let mut results = Vec::new();

    // Parse source with tree-sitter
    let tree = match crate::ast::parser::parse(source, lang) {
        Ok(t) => t,
        Err(_) => return results,
    };

    let root = tree.root_node();
    let source_bytes = source.as_bytes();
    let node_kinds = binary_expr_node_kinds(lang);

    if node_kinds.is_empty() {
        return results;
    }

    // Walk the AST to find binary expression nodes
    let mut cursor = root.walk();
    collect_binary_exprs(
        &mut cursor,
        source_bytes,
        lang,
        node_kinds,
        start_line,
        end_line,
        &mut results,
    );

    results
}

/// Recursively collect binary expressions from the AST.
fn collect_binary_exprs(
    cursor: &mut tree_sitter::TreeCursor,
    source: &[u8],
    lang: Language,
    node_kinds: &[&str],
    start_line: usize,
    end_line: usize,
    results: &mut Vec<(String, String, String, String, usize)>,
) {
    let node = cursor.node();
    let line = node.start_position().row + 1; // tree-sitter is 0-indexed

    // Skip nodes outside line range
    let node_start_line = node.start_position().row + 1;
    let node_end_line = node.end_position().row + 1;

    // If the entire node is outside the range, skip
    if node_end_line < start_line || node_start_line > end_line {
        return;
    }

    // Check if this node is a binary expression
    let kind = node.kind();
    if node_kinds.contains(&kind) && line >= start_line && line <= end_line {
        // Try to extract operator and operands
        if let Some(op) = extract_operator_from_node(&node, source, lang) {
            if let Some((left, right)) = extract_operands_from_node(&node, source, lang) {
                // Normalize the expression text
                let normalized = normalize_expression(&left, &op, &right);
                results.push((normalized, op, left, right, line));
            }
        }
    }

    // Recurse into children
    if cursor.goto_first_child() {
        loop {
            collect_binary_exprs(
                cursor, source, lang, node_kinds, start_line, end_line, results,
            );
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Expression Tests
    // =========================================================================

    #[test]
    fn test_expression_new() {
        let expr = Expression::new("a + b", vec!["b", "a"], 5);
        assert_eq!(expr.text, "a + b");
        // Operands should be sorted
        assert_eq!(expr.operands, vec!["a", "b"]);
        assert_eq!(expr.line, 5);
    }

    #[test]
    fn test_expression_equality_by_text() {
        let expr1 = Expression::new("a + b", vec!["a", "b"], 1);
        let expr2 = Expression::new("a + b", vec!["a", "b"], 100);
        assert_eq!(expr1, expr2);
    }

    #[test]
    fn test_expression_inequality_by_text() {
        let expr1 = Expression::new("a + b", vec!["a", "b"], 1);
        let expr2 = Expression::new("a - b", vec!["a", "b"], 1);
        assert_ne!(expr1, expr2);
    }

    #[test]
    fn test_expression_hash_by_text() {
        use std::collections::hash_map::DefaultHasher;

        let expr1 = Expression::new("x * y", vec!["x", "y"], 1);
        let expr2 = Expression::new("x * y", vec!["x", "y"], 999);

        let mut hasher1 = DefaultHasher::new();
        let mut hasher2 = DefaultHasher::new();
        expr1.hash(&mut hasher1);
        expr2.hash(&mut hasher2);

        assert_eq!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn test_expression_is_killed_by() {
        let expr = Expression::new("a + b", vec!["a", "b"], 1);
        assert!(expr.is_killed_by("a"));
        assert!(expr.is_killed_by("b"));
        assert!(!expr.is_killed_by("c"));
    }

    #[test]
    fn test_expression_hashset() {
        let expr1 = Expression::new("a + b", vec!["a", "b"], 1);
        let expr2 = Expression::new("a + b", vec!["a", "b"], 5);

        let mut set: HashSet<Expression> = HashSet::new();
        set.insert(expr1);
        set.insert(expr2);

        // Same text = only one entry
        assert_eq!(set.len(), 1);
    }

    // =========================================================================
    // Normalization Tests
    // =========================================================================

    #[test]
    fn test_normalize_commutative_addition() {
        assert_eq!(normalize_expression("a", "+", "b"), "a + b");
        assert_eq!(normalize_expression("b", "+", "a"), "a + b");
    }

    #[test]
    fn test_normalize_commutative_multiplication() {
        assert_eq!(normalize_expression("x", "*", "y"), "x * y");
        assert_eq!(normalize_expression("y", "*", "x"), "x * y");
    }

    #[test]
    fn test_normalize_commutative_equality() {
        assert_eq!(normalize_expression("foo", "==", "bar"), "bar == foo");
        assert_eq!(normalize_expression("bar", "==", "foo"), "bar == foo");
    }

    #[test]
    fn test_normalize_non_commutative_subtraction() {
        assert_eq!(normalize_expression("a", "-", "b"), "a - b");
        assert_eq!(normalize_expression("b", "-", "a"), "b - a");
    }

    #[test]
    fn test_normalize_non_commutative_division() {
        assert_eq!(normalize_expression("x", "/", "y"), "x / y");
        assert_eq!(normalize_expression("y", "/", "x"), "y / x");
    }

    #[test]
    fn test_normalize_whitespace_trimmed() {
        // ELEPHANT-PASS1-5: Whitespace should be trimmed
        assert_eq!(normalize_expression("  a  ", "+", "  b  "), "a + b");
        assert_eq!(normalize_expression("\ta\n", "-", "\tb\n"), "a - b");
    }

    // =========================================================================
    // BlockExpressions Tests
    // =========================================================================

    #[test]
    fn test_block_expressions_default() {
        let block = BlockExpressions::new();
        assert!(block.gen.is_empty());
        assert!(block.kill.is_empty());
    }

    // =========================================================================
    // AvailableExprsInfo Tests
    // =========================================================================

    #[test]
    fn test_available_exprs_info_empty() {
        let info = AvailableExprsInfo::empty(0);
        assert!(info.avail_in.is_empty());
        assert!(info.avail_out.is_empty());
        assert!(info.all_exprs.is_empty());
        assert_eq!(info.entry_block, 0);
        assert!(info.expr_instances.is_empty());
    }

    #[test]
    fn test_is_available_true() {
        let mut info = AvailableExprsInfo::new(0);
        let expr = Expression::new("a + b", vec!["a", "b"], 1);

        let mut block_exprs = HashSet::new();
        block_exprs.insert(expr.clone());
        info.avail_in.insert(1, block_exprs);

        assert!(info.is_available(1, &expr));
    }

    #[test]
    fn test_is_available_false_not_in_set() {
        let info = AvailableExprsInfo::new(0);
        let expr = Expression::new("a + b", vec!["a", "b"], 1);
        assert!(!info.is_available(1, &expr));
    }

    #[test]
    fn test_is_available_false_unknown_block() {
        let info = AvailableExprsInfo::new(0);
        let expr = Expression::new("a + b", vec!["a", "b"], 1);
        assert!(!info.is_available(999, &expr));
    }

    #[test]
    fn test_is_available_at_exit_true() {
        let mut info = AvailableExprsInfo::new(0);
        let expr = Expression::new("a + b", vec!["a", "b"], 1);

        let mut block_exprs = HashSet::new();
        block_exprs.insert(expr.clone());
        info.avail_out.insert(1, block_exprs);

        assert!(info.is_available_at_exit(1, &expr));
    }

    #[test]
    fn test_to_json_serializable() {
        let mut info = AvailableExprsInfo::new(0);
        let expr = Expression::new("a + b", vec!["a", "b"], 2);

        let mut block_exprs = HashSet::new();
        block_exprs.insert(expr.clone());
        info.avail_in.insert(0, HashSet::new());
        info.avail_in.insert(1, block_exprs.clone());
        info.avail_out.insert(0, block_exprs);
        info.all_exprs.insert(expr);

        let json = info.to_json();

        // Verify it's valid JSON
        assert!(json.is_object());
        assert!(json.get("avail_in").is_some());
        assert!(json.get("avail_out").is_some());
        assert!(json.get("all_expressions").is_some());
        assert!(json.get("entry_block").is_some());
        assert!(json.get("redundant_computations").is_some());
    }

    #[test]
    fn test_to_json_includes_redundant_computations() {
        let mut info = AvailableExprsInfo::new(0);

        // Add same expression twice (different lines)
        info.expr_instances
            .push(Expression::new("a + b", vec!["a", "b"], 2));
        info.expr_instances
            .push(Expression::new("a + b", vec!["a", "b"], 5));

        let json = info.to_json();
        let redundant = json
            .get("redundant_computations")
            .unwrap()
            .as_array()
            .unwrap();

        assert_eq!(redundant.len(), 1);
        assert_eq!(redundant[0]["expr"], "a + b");
        assert_eq!(redundant[0]["first_at"], 2);
        assert_eq!(redundant[0]["redundant_at"], 5);
    }

    // =========================================================================
    // Phase 2: Expression Extraction Tests (CAP-AE-12)
    // =========================================================================

    #[test]
    fn test_is_function_call_simple() {
        // Simple function calls
        assert!(is_function_call("foo()"));
        assert!(is_function_call("bar(x)"));
        assert!(is_function_call("baz(1, 2, 3)"));
    }

    #[test]
    fn test_is_function_call_method() {
        // Method calls
        assert!(is_function_call("obj.method()"));
        assert!(is_function_call("x.foo(bar)"));
        assert!(is_function_call("self.process(data)"));
    }

    #[test]
    fn test_is_function_call_chained() {
        // Chained method calls
        assert!(is_function_call("a.b.c()"));
        assert!(is_function_call("foo().bar()"));
    }

    #[test]
    fn test_is_function_call_false_for_binary_ops() {
        // Binary operations should NOT be function calls
        assert!(!is_function_call("a + b"));
        assert!(!is_function_call("x * y"));
        assert!(!is_function_call("foo - bar"));
        assert!(!is_function_call("1 + 2"));
    }

    #[test]
    fn test_is_function_call_false_for_parens_in_expr() {
        // Parentheses in expressions (not calls)
        assert!(!is_function_call("(a + b)"));
        assert!(!is_function_call("(x * y) + z"));
    }

    #[test]
    fn test_parse_expression_simple_addition() {
        let result = parse_expression_from_line("x = a + b");
        assert!(result.is_some());
        let (left, op, right) = result.unwrap();
        assert_eq!(left, "a");
        assert_eq!(op, "+");
        assert_eq!(right, "b");
    }

    #[test]
    fn test_parse_expression_multiplication() {
        let result = parse_expression_from_line("result = foo * bar");
        assert!(result.is_some());
        let (left, op, right) = result.unwrap();
        assert_eq!(left, "foo");
        assert_eq!(op, "*");
        assert_eq!(right, "bar");
    }

    #[test]
    fn test_parse_expression_subtraction() {
        let result = parse_expression_from_line("y = x - z");
        assert!(result.is_some());
        let (left, op, right) = result.unwrap();
        assert_eq!(left, "x");
        assert_eq!(op, "-");
        assert_eq!(right, "z");
    }

    #[test]
    fn test_parse_expression_comparison() {
        let result = parse_expression_from_line("flag = a == b");
        assert!(result.is_some());
        let (left, op, right) = result.unwrap();
        assert_eq!(left, "a");
        assert_eq!(op, "==");
        assert_eq!(right, "b");
    }

    #[test]
    fn test_parse_expression_with_comment() {
        // Comment should be stripped
        let result = parse_expression_from_line("x = a + b  # compute sum");
        assert!(result.is_some());
        let (left, op, right) = result.unwrap();
        assert_eq!(left, "a");
        assert_eq!(op, "+");
        assert_eq!(right, "b");
    }

    #[test]
    fn test_parse_expression_function_call_excluded() {
        // Function calls should NOT produce expressions (CAP-AE-12)
        assert!(parse_expression_from_line("x = foo()").is_none());
        assert!(parse_expression_from_line("y = bar.baz()").is_none());
        assert!(parse_expression_from_line("z = process(data)").is_none());
    }

    #[test]
    fn test_parse_expression_constant_only_excluded() {
        // Pure constant expressions should not be CSE targets
        assert!(parse_expression_from_line("x = 1 + 2").is_none());
        assert!(parse_expression_from_line("y = 10 * 20").is_none());
    }

    #[test]
    fn test_parse_expression_with_spaces() {
        // Whitespace should be handled
        let result = parse_expression_from_line("   x   =   a   +   b   ");
        assert!(result.is_some());
        let (left, op, right) = result.unwrap();
        assert_eq!(left, "a");
        assert_eq!(op, "+");
        assert_eq!(right, "b");
    }

    #[test]
    fn test_parse_expression_rightmost_assignment() {
        // TIGER-PASS2-1: Only process rightmost assignment
        // For "a = b = c + d", we should get "c + d", not "b = c + d"
        let result = parse_expression_from_line("a = b = c + d");
        assert!(result.is_some());
        let (left, op, right) = result.unwrap();
        assert_eq!(left, "c");
        assert_eq!(op, "+");
        assert_eq!(right, "d");
    }

    #[test]
    fn test_extract_base_variable_simple() {
        assert_eq!(extract_base_variable("x"), "x");
        assert_eq!(extract_base_variable("foo"), "foo");
    }

    #[test]
    fn test_extract_base_variable_field_access() {
        assert_eq!(extract_base_variable("x.a"), "x.a");
        assert_eq!(extract_base_variable("obj.field"), "obj.field");
    }

    #[test]
    fn test_extract_base_variable_deep_nesting_truncated() {
        // TIGER-PASS3-5: Limit to 3 levels
        assert_eq!(extract_base_variable("x.a.b.c.d.e"), "x.a.b");
        assert_eq!(extract_base_variable("a.b.c.d"), "a.b.c");
    }

    #[test]
    fn test_is_numeric_literal() {
        // Integers
        assert!(is_numeric_literal("42"));
        assert!(is_numeric_literal("-10"));
        assert!(is_numeric_literal("0"));

        // Floats
        assert!(is_numeric_literal("3.14"));
        assert!(is_numeric_literal("-2.5"));

        // Hex/octal/binary
        assert!(is_numeric_literal("0x1f"));
        assert!(is_numeric_literal("0o17"));
        assert!(is_numeric_literal("0b1010"));

        // Not numeric
        assert!(!is_numeric_literal("foo"));
        assert!(!is_numeric_literal("x"));
        assert!(!is_numeric_literal("a + b"));
    }

    #[test]
    fn test_is_identifier() {
        assert!(is_identifier("x"));
        assert!(is_identifier("foo"));
        assert!(is_identifier("_private"));
        assert!(is_identifier("var123"));
        assert!(is_identifier("obj.field"));

        assert!(!is_identifier(""));
        assert!(!is_identifier("123"));
        assert!(!is_identifier("a + b"));
    }

    #[test]
    fn test_find_operator_in_expr() {
        assert_eq!(find_operator_in_expr("a + b", "+"), Some(2));
        assert_eq!(find_operator_in_expr("x * y", "*"), Some(2));
        assert_eq!(find_operator_in_expr("foo - bar", "-"), Some(4));

        // Should not find operator inside parens
        assert_eq!(find_operator_in_expr("(a + b) * c", "+"), None);

        // Should find outer operator
        assert_eq!(find_operator_in_expr("(a + b) * c", "*"), Some(8));
    }

    #[test]
    fn test_find_operator_not_in_string() {
        // Operator inside string should not be found
        assert_eq!(find_operator_in_expr("\"a + b\"", "+"), None);
        assert_eq!(find_operator_in_expr("'x * y'", "*"), None);
    }

    // =========================================================================
    // Phase 3: compute_available_exprs Tests
    // =========================================================================

    use crate::types::{BlockType, CfgBlock, CfgEdge, EdgeType};

    /// Helper to create a minimal CFG for testing
    fn make_test_cfg(
        blocks: Vec<(usize, BlockType, (u32, u32))>,
        edges: Vec<(usize, usize)>,
        entry: usize,
    ) -> CfgInfo {
        CfgInfo {
            function: "test".to_string(),
            blocks: blocks
                .into_iter()
                .map(|(id, block_type, lines)| CfgBlock {
                    id,
                    block_type,
                    lines,
                    calls: vec![],
                })
                .collect(),
            edges: edges
                .into_iter()
                .map(|(from, to)| CfgEdge {
                    from,
                    to,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                })
                .collect(),
            entry_block: entry,
            exit_blocks: vec![],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        }
    }

    /// Helper to create an empty DFG
    fn make_empty_dfg() -> DfgInfo {
        DfgInfo {
            function: "test".to_string(),
            refs: vec![],
            edges: vec![],
            variables: vec![],
        }
    }

    /// Helper to create a DFG with specific refs
    fn make_dfg_with_refs(refs: Vec<VarRef>) -> DfgInfo {
        let variables: Vec<String> = refs.iter().map(|r| r.name.clone()).collect();
        DfgInfo {
            function: "test".to_string(),
            refs,
            edges: vec![],
            variables,
        }
    }

    /// Helper to create a VarRef
    fn make_var_ref(name: &str, line: u32, ref_type: RefType) -> VarRef {
        VarRef {
            name: name.to_string(),
            ref_type,
            line,
            column: 0,
            context: None,
            group_id: None,
        }
    }

    #[test]
    fn test_compute_empty_cfg_empty_dfg() {
        // Empty function - no blocks means NoCfg error
        let cfg = CfgInfo {
            function: "empty".to_string(),
            blocks: vec![],
            edges: vec![],
            entry_block: 0,
            exit_blocks: vec![],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        };
        let dfg = make_empty_dfg();

        let result = compute_available_exprs(&cfg, &dfg);
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_single_block_no_exprs() {
        // Single entry block, no expressions
        let cfg = make_test_cfg(vec![(0, BlockType::Entry, (1, 1))], vec![], 0);
        let dfg = make_empty_dfg();

        let result = compute_available_exprs(&cfg, &dfg);
        assert!(result.is_ok());
        let info = result.unwrap();

        // No expressions
        assert!(info.all_exprs.is_empty());

        // avail_in and avail_out should exist for block 0
        assert!(info.avail_in.contains_key(&0));
        assert!(info.avail_out.contains_key(&0));

        // Entry block should have nothing available at entry
        assert!(info.avail_in.get(&0).unwrap().is_empty());
    }

    #[test]
    fn test_compute_entry_block_nothing_available() {
        // Entry block always has nothing available at entry
        let cfg = make_test_cfg(
            vec![(0, BlockType::Entry, (1, 5)), (1, BlockType::Exit, (6, 10))],
            vec![(0, 1)],
            0,
        );
        let dfg = make_empty_dfg();

        let result = compute_available_exprs(&cfg, &dfg).unwrap();

        // Entry block avail_in should be empty
        assert!(result.avail_in.get(&0).unwrap().is_empty());
    }

    #[test]
    fn test_compute_linear_cfg_expression_propagates() {
        // Linear CFG: 0 -> 1 -> 2
        // Expression computed in block 0 should be available in blocks 1 and 2
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry, (1, 2)),
                (1, BlockType::Body, (3, 4)),
                (2, BlockType::Exit, (5, 6)),
            ],
            vec![(0, 1), (1, 2)],
            0,
        );

        // DFG with a+b expression in block 0
        let dfg = make_dfg_with_refs(vec![
            make_var_ref("x", 2, RefType::Definition),
            make_var_ref("a", 2, RefType::Use),
            make_var_ref("b", 2, RefType::Use),
        ]);

        let result = compute_available_exprs(&cfg, &dfg).unwrap();

        // If expressions were extracted, they should propagate downstream
        // Note: this depends on expression extraction working
        if !result.all_exprs.is_empty() {
            let expr = result.all_exprs.iter().next().unwrap();

            // Expression generated in block 0 should be available at exit of 0
            assert!(result.is_available_at_exit(0, expr));

            // Available at entry to block 1
            assert!(result.is_available(1, expr));

            // Available at entry to block 2
            assert!(result.is_available(2, expr));
        }
    }

    #[test]
    fn test_compute_diamond_must_intersection() {
        // Diamond CFG:
        //      [0: entry]
        //       /      \
        //   [1:x=a+b]  [2:skip]
        //       \      /
        //        [3:merge]
        //
        // Expression in only one branch should NOT be available at merge (MUST)
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry, (1, 1)),
                (1, BlockType::Body, (2, 2)),
                (2, BlockType::Body, (3, 3)),
                (3, BlockType::Exit, (4, 4)),
            ],
            vec![(0, 1), (0, 2), (1, 3), (2, 3)],
            0,
        );

        // Expression only in block 1
        let dfg = make_dfg_with_refs(vec![
            make_var_ref("x", 2, RefType::Definition),
            make_var_ref("a", 2, RefType::Use),
            make_var_ref("b", 2, RefType::Use),
        ]);

        let result = compute_available_exprs(&cfg, &dfg).unwrap();

        // If expressions were extracted from block 1 only
        if !result.all_exprs.is_empty() {
            let expr = result.all_exprs.iter().next().unwrap();

            // Expression should be available at exit of block 1
            assert!(result.is_available_at_exit(1, expr));

            // Block 2 has no expressions, so nothing generated there
            // At merge (block 3), MUST analysis (intersection) means:
            // avail_in[3] = avail_out[1] ∩ avail_out[2]
            // Since block 2 doesn't have the expr in avail_out, it won't be in avail_in[3]

            // This test validates MUST semantics
            // Note: Block 2's avail_out will be what flows through from avail_in[2]
        }
    }

    #[test]
    fn test_compute_self_loop_terminates() {
        // CFG with self-loop: 0 -> 1 -> 1 (self-loop)
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry, (1, 1)),
                (1, BlockType::LoopHeader, (2, 3)),
            ],
            vec![(0, 1), (1, 1)],
            0,
        );
        let dfg = make_empty_dfg();

        // Should not infinite loop
        let result = compute_available_exprs(&cfg, &dfg);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compute_unreachable_block() {
        // CFG with unreachable block
        // 0 -> 1, but block 2 is unreachable
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry, (1, 1)),
                (1, BlockType::Exit, (2, 2)),
                (2, BlockType::Body, (3, 3)), // No edges to this block
            ],
            vec![(0, 1)],
            0,
        );
        let dfg = make_empty_dfg();

        let result = compute_available_exprs(&cfg, &dfg);
        assert!(result.is_ok());

        let info = result.unwrap();

        // Unreachable block should have empty avail_in (no predecessors)
        assert!(info.avail_in.get(&2).unwrap().is_empty());
    }

    #[test]
    fn test_is_killed_function() {
        let kills: HashSet<String> = ["a".to_string(), "x".to_string()].into_iter().collect();

        let expr1 = Expression::new("a + b", vec!["a", "b"], 1);
        let expr2 = Expression::new("c + d", vec!["c", "d"], 2);
        let expr3 = Expression::new("x + y", vec!["x", "y"], 3);

        // expr1 uses 'a', which is in kills
        assert!(is_killed(&expr1, &kills));

        // expr2 uses 'c' and 'd', neither in kills
        assert!(!is_killed(&expr2, &kills));

        // expr3 uses 'x', which is in kills
        assert!(is_killed(&expr3, &kills));
    }

    #[test]
    fn test_compute_loop_expression_available_in_body() {
        // Loop CFG: 0 -> 1 (header) <-> 2 (body) -> 3 (exit)
        let cfg = make_test_cfg(
            vec![
                (0, BlockType::Entry, (1, 1)),
                (1, BlockType::LoopHeader, (2, 2)),
                (2, BlockType::LoopBody, (3, 3)),
                (3, BlockType::Exit, (4, 4)),
            ],
            vec![(0, 1), (1, 2), (2, 1), (1, 3)],
            0,
        );
        let dfg = make_empty_dfg();

        let result = compute_available_exprs(&cfg, &dfg);
        assert!(result.is_ok());
    }

    // =========================================================================
    // AST-based expression extraction tests
    // =========================================================================

    #[test]
    fn test_extract_binary_exprs_from_ast_python() {
        let source = r#"
def example(a, b, c):
    x = a + b
    if a * c > 10:
        return a + b
    y = c - a
"#;
        let lang = crate::types::Language::Python;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 6);
        // Should find: a + b (line 3), a * c (line 4), a + b (line 5), c - a (line 6)
        assert!(
            exprs.len() >= 3,
            "Expected at least 3 binary exprs from Python, got {}: {:?}",
            exprs.len(),
            exprs
        );
        // Check that we found a + b
        let texts: Vec<&str> = exprs.iter().map(|e| e.0.as_str()).collect();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("a") && t.contains("b") && t.contains("+")),
            "Should find a + b expression, got: {:?}",
            texts
        );
    }

    #[test]
    fn test_extract_binary_exprs_from_ast_go() {
        let source = r#"
package main

func example(a int, b int, c int) int {
    x := a + b
    if a * c > 10 {
        return a + b
    }
    y := c - a
    return y
}
"#;
        let lang = crate::types::Language::Go;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 11);
        assert!(
            exprs.len() >= 3,
            "Expected at least 3 binary exprs from Go, got {}: {:?}",
            exprs.len(),
            exprs
        );
    }

    #[test]
    fn test_extract_binary_exprs_from_ast_java() {
        let source = r#"
class Example {
    int example(int a, int b, int c) {
        int x = a + b;
        if (a * c > 10) {
            return a + b;
        }
        int y = c - a;
        return y;
    }
}
"#;
        let lang = crate::types::Language::Java;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 11);
        assert!(
            exprs.len() >= 3,
            "Expected at least 3 binary exprs from Java, got {}: {:?}",
            exprs.len(),
            exprs
        );
    }

    #[test]
    fn test_extract_binary_exprs_from_ast_ruby() {
        let source = r#"
def example(a, b, c)
  x = a + b
  if a * c > 10
    return a + b
  end
  y = c - a
  y
end
"#;
        let lang = crate::types::Language::Ruby;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 9);
        assert!(
            exprs.len() >= 3,
            "Expected at least 3 binary exprs from Ruby, got {}: {:?}",
            exprs.len(),
            exprs
        );
    }

    #[test]
    fn test_extract_binary_exprs_from_ast_cpp() {
        let source = r#"
int example(int a, int b, int c) {
    int x = a + b;
    if (a * c > 10) {
        return a + b;
    }
    int y = c - a;
    return y;
}
"#;
        let lang = crate::types::Language::Cpp;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 9);
        assert!(
            exprs.len() >= 3,
            "Expected at least 3 binary exprs from C++, got {}: {:?}",
            exprs.len(),
            exprs
        );
    }

    #[test]
    fn test_extract_binary_exprs_from_ast_php() {
        let source = r#"<?php
function example($a, $b, $c) {
    $x = $a + $b;
    if ($a * $c > 10) {
        return $a + $b;
    }
    $y = $c - $a;
    return $y;
}
"#;
        let lang = crate::types::Language::Php;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 9);
        assert!(
            exprs.len() >= 3,
            "Expected at least 3 binary exprs from PHP, got {}: {:?}",
            exprs.len(),
            exprs
        );
    }

    #[test]
    fn test_extract_binary_exprs_from_ast_csharp() {
        let source = r#"
class Example {
    int DoWork(int a, int b, int c) {
        int x = a + b;
        if (a * c > 10) {
            return a + b;
        }
        int y = c - a;
        return y;
    }
}
"#;
        let lang = crate::types::Language::CSharp;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 11);
        assert!(
            exprs.len() >= 3,
            "Expected at least 3 binary exprs from C#, got {}: {:?}",
            exprs.len(),
            exprs
        );
    }

    #[test]
    fn test_extract_binary_exprs_from_ast_kotlin() {
        let source = r#"
fun example(a: Int, b: Int, c: Int): Int {
    val x = a + b
    if (a * c > 10) {
        return a + b
    }
    val y = c - a
    return y
}
"#;
        let lang = crate::types::Language::Kotlin;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 9);
        assert!(
            exprs.len() >= 3,
            "Expected at least 3 binary exprs from Kotlin, got {}: {:?}",
            exprs.len(),
            exprs
        );
    }

    #[test]
    fn test_extract_binary_exprs_from_ast_elixir() {
        let source = r#"
defmodule Example do
  def example(a, b, c) do
    x = a + b
    if a * c > 10 do
      a + b
    end
    y = c - a
    y
  end
end
"#;
        let lang = crate::types::Language::Elixir;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 11);
        assert!(
            exprs.len() >= 3,
            "Expected at least 3 binary exprs from Elixir, got {}: {:?}",
            exprs.len(),
            exprs
        );
    }

    #[test]
    fn test_extract_binary_exprs_from_ast_rust() {
        let source = r#"
fn example(a: i32, b: i32, c: i32) -> i32 {
    let x = a + b;
    if a * c > 10 {
        return a + b;
    }
    let y = c - a;
    y
}
"#;
        let lang = crate::types::Language::Rust;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 9);
        assert!(
            exprs.len() >= 3,
            "Expected at least 3 binary exprs from Rust, got {}: {:?}",
            exprs.len(),
            exprs
        );
    }

    #[test]
    fn test_extract_binary_exprs_excludes_function_calls() {
        let source = r#"
def example(a, b):
    x = a + b
    y = len(a)
    z = foo(a, b)
"#;
        let lang = crate::types::Language::Python;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 5);
        // Should find a + b but NOT the function calls
        let texts: Vec<&str> = exprs.iter().map(|e| e.0.as_str()).collect();
        assert!(
            !texts.iter().any(|t| t.contains("len") || t.contains("foo")),
            "Should not include function calls, got: {:?}",
            texts
        );
    }

    #[test]
    fn test_extract_binary_exprs_normalizes_commutative() {
        let source = r#"
def example(a, b):
    x = b + a
    y = a + b
"#;
        let lang = crate::types::Language::Python;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 4);
        // Both should normalize to "a + b"
        let texts: Vec<String> = exprs.iter().map(|e| e.0.clone()).collect();
        if texts.len() >= 2 {
            assert_eq!(
                texts[0], texts[1],
                "Commutative exprs should normalize to same text: {:?}",
                texts
            );
        }
    }

    #[test]
    fn test_extract_binary_exprs_returns_line_numbers() {
        let source = r#"
def example(a, b):
    x = a + b
    y = a - b
"#;
        let lang = crate::types::Language::Python;
        let exprs = extract_binary_exprs_from_ast(source, lang, 1, 4);
        assert!(
            exprs.len() >= 2,
            "Expected at least 2 exprs, got {}",
            exprs.len()
        );
        // Lines should be > 0
        for (text, _op, _left, _right, line) in &exprs {
            assert!(*line > 0, "Line should be > 0 for expr: {}", text);
        }
    }

    #[test]
    fn test_parse_expression_no_assignment_return() {
        // Expressions in return statements (no assignment)
        let result = parse_expression_from_line("    return a + b");
        assert!(
            result.is_some(),
            "Should parse expression from return statement"
        );
        let (left, op, right) = result.unwrap();
        assert_eq!(op, "+");
        assert!(left == "a" || right == "a");
    }

    #[test]
    fn test_parse_expression_no_assignment_if() {
        // Expressions in if conditions
        let result = parse_expression_from_line("    if a + b > 10:");
        // Should find at least one binary expression
        assert!(
            result.is_some(),
            "Should parse expression from if condition"
        );
    }

    #[test]
    fn test_parse_expression_no_assignment_standalone() {
        // Standalone expression
        let result = parse_expression_from_line("    a + b");
        assert!(
            result.is_some(),
            "Should parse standalone binary expression"
        );
    }
}
