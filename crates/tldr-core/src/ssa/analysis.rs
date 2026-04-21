//! SSA-Based Analyses
//!
//! Provides analyses that leverage SSA form:
//! - Value Numbering (Global Value Numbering)
//! - SCCP (Sparse Conditional Constant Propagation)
//! - Dead Code Detection
//! - Live Variables Analysis
//! - Available Expressions

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::types::{CfgInfo, Language};
use crate::TldrResult;

use super::types::{SsaFunction, SsaNameId};

// =============================================================================
// Live Variables Analysis
// =============================================================================

/// Live variables at each program point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveVariables {
    /// Function name
    pub function: String,
    /// For each block, variables live at entry and exit
    pub blocks: HashMap<usize, LiveSets>,
}

/// Live variable sets for a block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSets {
    /// Variables live at block entry
    pub live_in: HashSet<String>,
    /// Variables live at block exit
    pub live_out: HashSet<String>,
}

/// Compute live variables (backward may-analysis)
///
/// # Dataflow Equations
/// ```text
/// OUT[B] = union IN[S] for all successors S
/// IN[B] = USE[B] union (OUT[B] - DEF[B])
///
/// USE[B] = variables used before defined in B
/// DEF[B] = variables defined in B
/// ```
///
/// # Algorithm
/// 1. Build line-to-block mapping
/// 2. Compute USE and DEF sets for each block
/// 3. Iterate backward dataflow until fixed point
pub fn compute_live_variables(
    cfg: &CfgInfo,
    refs: &[crate::types::VarRef],
) -> TldrResult<LiveVariables> {
    use crate::types::RefType;

    // Build line-to-block mapping
    let line_to_block: HashMap<u32, usize> = cfg
        .blocks
        .iter()
        .flat_map(|block| (block.lines.0..=block.lines.1).map(move |line| (line, block.id)))
        .collect();

    // Build successor map for looking up successors by block ID
    let mut successors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        successors.entry(block.id).or_default();
    }
    for edge in &cfg.edges {
        successors.entry(edge.from).or_default().push(edge.to);
    }

    // Compute USE and DEF sets for each block
    // USE[B] = variables used before being defined in B
    // DEF[B] = variables defined in B
    let mut use_sets: HashMap<usize, HashSet<String>> = HashMap::new();
    let mut def_sets: HashMap<usize, HashSet<String>> = HashMap::new();

    for block in &cfg.blocks {
        use_sets.insert(block.id, HashSet::new());
        def_sets.insert(block.id, HashSet::new());
    }

    // Group refs by block
    let mut block_refs: HashMap<usize, Vec<&crate::types::VarRef>> = HashMap::new();
    for var_ref in refs {
        if let Some(&block_id) = line_to_block.get(&var_ref.line) {
            block_refs.entry(block_id).or_default().push(var_ref);
        }
    }

    // Process each block to compute USE and DEF
    for (&block_id, refs_in_block) in &block_refs {
        // Sort refs by line number to process in order
        let mut sorted_refs: Vec<_> = refs_in_block.iter().collect();
        sorted_refs.sort_by_key(|r| r.line);

        let use_set = use_sets.get_mut(&block_id).unwrap();
        let def_set = def_sets.get_mut(&block_id).unwrap();

        for var_ref in sorted_refs {
            match var_ref.ref_type {
                RefType::Use => {
                    // If not already defined in this block, it's a USE
                    if !def_set.contains(&var_ref.name) {
                        use_set.insert(var_ref.name.clone());
                    }
                }
                RefType::Definition => {
                    def_set.insert(var_ref.name.clone());
                }
                RefType::Update => {
                    // Update is USE then DEF
                    // First, it's a use if not already defined
                    if !def_set.contains(&var_ref.name) {
                        use_set.insert(var_ref.name.clone());
                    }
                    // Then it's a definition
                    def_set.insert(var_ref.name.clone());
                }
            }
        }
    }

    // Initialize live_in and live_out sets
    let mut live_in: HashMap<usize, HashSet<String>> = HashMap::new();
    let mut live_out: HashMap<usize, HashSet<String>> = HashMap::new();

    for block in &cfg.blocks {
        live_in.insert(block.id, HashSet::new());
        live_out.insert(block.id, HashSet::new());
    }

    // Backward dataflow iteration until fixed point
    // Process in reverse topological order for faster convergence
    let block_ids: Vec<usize> = cfg.blocks.iter().map(|b| b.id).collect();
    let max_iterations = block_ids.len() * 2 + 10; // Safety limit

    let mut changed = true;
    let mut iterations = 0;

    while changed && iterations < max_iterations {
        changed = false;
        iterations += 1;

        // Process blocks in reverse order (approximation of reverse postorder)
        for &block_id in block_ids.iter().rev() {
            // OUT[B] = union of IN[S] for all successors S
            let mut new_out: HashSet<String> = HashSet::new();
            if let Some(succs) = successors.get(&block_id) {
                for &succ_id in succs {
                    if let Some(succ_in) = live_in.get(&succ_id) {
                        new_out.extend(succ_in.iter().cloned());
                    }
                }
            }

            // IN[B] = USE[B] union (OUT[B] - DEF[B])
            let use_b = use_sets.get(&block_id).cloned().unwrap_or_default();
            let def_b = def_sets.get(&block_id).cloned().unwrap_or_default();

            let out_minus_def: HashSet<String> = new_out
                .difference(&def_b)
                .cloned()
                .collect();

            let mut new_in = use_b;
            new_in.extend(out_minus_def);

            // Check for changes
            if &new_in != live_in.get(&block_id).unwrap() {
                changed = true;
                live_in.insert(block_id, new_in);
            }
            if &new_out != live_out.get(&block_id).unwrap() {
                changed = true;
                live_out.insert(block_id, new_out);
            }
        }
    }

    // Build result
    let mut blocks_result = HashMap::new();
    for block in &cfg.blocks {
        blocks_result.insert(
            block.id,
            LiveSets {
                live_in: live_in.get(&block.id).cloned().unwrap_or_default(),
                live_out: live_out.get(&block.id).cloned().unwrap_or_default(),
            },
        );
    }

    Ok(LiveVariables {
        function: cfg.function.clone(),
        blocks: blocks_result,
    })
}

/// Check if a variable is live at the entry of a block
impl LiveVariables {
    /// Returns true if the variable is live at the entry of the given block
    pub fn is_live_in(&self, block: usize, var: &str) -> bool {
        self.blocks
            .get(&block)
            .map(|sets| sets.live_in.contains(var))
            .unwrap_or(false)
    }

    /// Returns true if the variable is live at the exit of the given block
    pub fn is_live_out(&self, block: usize, var: &str) -> bool {
        self.blocks
            .get(&block)
            .map(|sets| sets.live_out.contains(var))
            .unwrap_or(false)
    }
}

// =============================================================================
// Value Numbering
// =============================================================================

/// Value numbering result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueNumbering {
    /// Function name
    pub function: String,
    /// Value number for each SSA name
    pub value_numbers: HashMap<SsaNameId, u32>,
    /// SSA names with the same value number (potential CSE)
    pub equivalences: HashMap<u32, Vec<SsaNameId>>,
}

/// Canonical expression for value numbering
/// Expressions are normalized for comparison (e.g., commutative ops sorted)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CanonicalExpr {
    /// Parameter or constant with no operands
    Leaf(String),
    /// Binary operation with canonicalized operands (sorted for commutative ops)
    BinaryOp {
        op: String,
        /// Value numbers of operands (sorted for commutative ops)
        operands: (u32, u32),
    },
    /// Unary operation
    UnaryOp {
        op: String,
        operand: u32,
    },
    /// Phi function with value numbers of sources
    Phi(Vec<u32>),
    /// Call (each call gets unique number - not CSE-able)
    Call(u32),
}

/// Compute value numbers for SSA names
///
/// # Algorithm (Global Value Numbering)
/// ```text
/// For each SSA name n:
///   expr = canonical expression computing n
///   if expr in hash_table:
///     value_number[n] = hash_table[expr]
///   else:
///     value_number[n] = next_number++
///     hash_table[expr] = value_number[n]
/// ```
///
/// # Postconditions
/// - Expressions with same operands get same value number
/// - `equivalences` maps value number to all equivalent SSA names
pub fn compute_value_numbers(ssa: &SsaFunction) -> TldrResult<ValueNumbering> {
    use super::types::SsaInstructionKind;

    let mut value_numbers: HashMap<SsaNameId, u32> = HashMap::new();
    let mut expr_to_number: HashMap<CanonicalExpr, u32> = HashMap::new();
    let mut next_number: u32 = 0;
    let mut call_counter: u32 = 0;

    // Helper to get or create value number for an expression
    let get_or_create_number = |expr: CanonicalExpr,
                                    expr_to_num: &mut HashMap<CanonicalExpr, u32>,
                                    next_num: &mut u32|
     -> u32 {
        if let Some(&num) = expr_to_num.get(&expr) {
            num
        } else {
            let num = *next_num;
            *next_num += 1;
            expr_to_num.insert(expr, num);
            num
        }
    };

    // Process blocks in dominator tree order (approximated by block ID order)
    // This ensures we see definitions before uses
    for block in &ssa.blocks {
        // Process phi functions first
        for phi in &block.phi_functions {
            // Get value numbers of sources (if available)
            let source_numbers: Vec<u32> = phi
                .sources
                .iter()
                .filter_map(|s| value_numbers.get(&s.name).copied())
                .collect();

            // Create canonical expression for phi
            let expr = CanonicalExpr::Phi(source_numbers);
            let vn = get_or_create_number(expr, &mut expr_to_number, &mut next_number);
            value_numbers.insert(phi.target, vn);
        }

        // Process instructions
        for instr in &block.instructions {
            if let Some(target) = instr.target {
                // Get value numbers of operands
                let use_numbers: Vec<u32> = instr
                    .uses
                    .iter()
                    .filter_map(|u| value_numbers.get(u).copied())
                    .collect();

                // Create canonical expression based on instruction kind
                let expr = match &instr.kind {
                    SsaInstructionKind::Param | SsaInstructionKind::Assign => {
                        if use_numbers.is_empty() {
                            // Constant or parameter - unique value based on source text
                            let key = instr
                                .source_text
                                .clone()
                                .unwrap_or_else(|| format!("const_{}", target.0));
                            CanonicalExpr::Leaf(key)
                        } else if use_numbers.len() == 1 {
                            // Simple assignment x = y - same value number as source
                            let source_vn = use_numbers[0];
                            // Reuse source value number directly
                            value_numbers.insert(target, source_vn);
                            continue;
                        } else {
                            // Multiple uses - treat as leaf with unique key
                            CanonicalExpr::Leaf(format!("assign_{}", target.0))
                        }
                    }
                    SsaInstructionKind::BinaryOp => {
                        if use_numbers.len() >= 2 {
                            let (left, right) = (use_numbers[0], use_numbers[1]);
                            // Canonicalize commutative operations (sort operands)
                            let op_name = instr
                                .source_text
                                .as_ref()
                                .and_then(|s| {
                                    // Extract operator from source text like "x = a + b"
                                    if s.contains('+') {
                                        Some("+")
                                    } else if s.contains('*') {
                                        Some("*")
                                    } else if s.contains('-') {
                                        Some("-")
                                    } else if s.contains('/') {
                                        Some("/")
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or("binop");

                            // Sort operands for commutative operations
                            let is_commutative = op_name == "+" || op_name == "*";
                            let operands = if is_commutative && left > right {
                                (right, left)
                            } else {
                                (left, right)
                            };

                            CanonicalExpr::BinaryOp {
                                op: op_name.to_string(),
                                operands,
                            }
                        } else {
                            CanonicalExpr::Leaf(format!("binop_{}", target.0))
                        }
                    }
                    SsaInstructionKind::UnaryOp => {
                        if !use_numbers.is_empty() {
                            CanonicalExpr::UnaryOp {
                                op: "unary".to_string(),
                                operand: use_numbers[0],
                            }
                        } else {
                            CanonicalExpr::Leaf(format!("unary_{}", target.0))
                        }
                    }
                    SsaInstructionKind::Call => {
                        // Each call gets a unique value number (not CSE-able due to side effects)
                        call_counter += 1;
                        CanonicalExpr::Call(call_counter)
                    }
                    SsaInstructionKind::Return | SsaInstructionKind::Branch => {
                        // These typically don't define values
                        continue;
                    }
                };

                let vn = get_or_create_number(expr, &mut expr_to_number, &mut next_number);
                value_numbers.insert(target, vn);
            }
        }
    }

    // Build equivalences map (group SSA names by value number)
    let mut equivalences: HashMap<u32, Vec<SsaNameId>> = HashMap::new();
    for (&name, &vn) in &value_numbers {
        equivalences.entry(vn).or_default().push(name);
    }

    Ok(ValueNumbering {
        function: ssa.function.clone(),
        value_numbers,
        equivalences,
    })
}

// =============================================================================
// SCCP (Sparse Conditional Constant Propagation)
// =============================================================================

/// SCCP lattice value
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LatticeValue {
    /// Unknown (top) - not yet analyzed
    Top,
    /// Constant value
    Constant(ConstantValue),
    /// Varying (bottom) - not constant
    Bottom,
}

/// Constant value representation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConstantValue {
    /// Integer constant
    Int(i64),
    /// Float constant (as string to avoid comparison issues)
    Float(String),
    /// String constant
    String(String),
    /// Boolean constant
    Bool(bool),
    /// None/null constant
    None,
}

impl std::fmt::Display for ConstantValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstantValue::Int(i) => write!(f, "{}", i),
            ConstantValue::Float(fl) => write!(f, "{}", fl),
            ConstantValue::String(s) => write!(f, "\"{}\"", s),
            ConstantValue::Bool(b) => write!(f, "{}", b),
            ConstantValue::None => write!(f, "None"),
        }
    }
}

/// SCCP analysis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SccpResult {
    /// Function name
    pub function: String,
    /// Constant values for SSA names
    pub constants: HashMap<SsaNameId, ConstantValue>,
    /// Unreachable blocks
    pub unreachable_blocks: HashSet<usize>,
    /// Dead SSA names (definitions never used)
    pub dead_names: HashSet<SsaNameId>,
}

/// Lattice meet operation: combines two lattice values
fn lattice_meet(a: &LatticeValue, b: &LatticeValue) -> LatticeValue {
    match (a, b) {
        // Top meet X = X
        (LatticeValue::Top, x) | (x, LatticeValue::Top) => x.clone(),
        // C meet C = C (if same constant)
        (LatticeValue::Constant(c1), LatticeValue::Constant(c2)) if c1 == c2 => {
            LatticeValue::Constant(c1.clone())
        }
        // Otherwise = Bottom
        _ => LatticeValue::Bottom,
    }
}

/// Try to parse a constant value from source text
fn parse_constant(source_text: &str) -> Option<ConstantValue> {
    let trimmed = source_text.trim();

    // Try to extract RHS of assignment (e.g., "x = 1" -> "1")
    let value_part = if let Some(idx) = trimmed.find('=') {
        trimmed[idx + 1..].trim()
    } else {
        trimmed
    };

    // Parse boolean
    if value_part == "True" || value_part == "true" {
        return Some(ConstantValue::Bool(true));
    }
    if value_part == "False" || value_part == "false" {
        return Some(ConstantValue::Bool(false));
    }

    // Parse None/null
    if value_part == "None" || value_part == "null" || value_part == "nil" {
        return Some(ConstantValue::None);
    }

    // Parse integer
    if let Ok(i) = value_part.parse::<i64>() {
        return Some(ConstantValue::Int(i));
    }

    // Parse float (as string to avoid comparison issues)
    if value_part.contains('.') && value_part.parse::<f64>().is_ok() {
        return Some(ConstantValue::Float(value_part.to_string()));
    }

    // Parse string literal
    if (value_part.starts_with('"') && value_part.ends_with('"'))
        || (value_part.starts_with('\'') && value_part.ends_with('\''))
    {
        let inner = &value_part[1..value_part.len() - 1];
        return Some(ConstantValue::String(inner.to_string()));
    }

    None
}

/// Evaluate a binary operation on constants
fn eval_binary_op(op: &str, left: &ConstantValue, right: &ConstantValue) -> Option<ConstantValue> {
    match (left, right) {
        (ConstantValue::Int(l), ConstantValue::Int(r)) => {
            let result = match op {
                "+" => l.checked_add(*r)?,
                "-" => l.checked_sub(*r)?,
                "*" => l.checked_mul(*r)?,
                "/" => {
                    if *r == 0 {
                        return None;
                    }
                    l.checked_div(*r)?
                }
                "%" => {
                    if *r == 0 {
                        return None;
                    }
                    l.checked_rem(*r)?
                }
                "<" => return Some(ConstantValue::Bool(l < r)),
                ">" => return Some(ConstantValue::Bool(l > r)),
                "<=" => return Some(ConstantValue::Bool(l <= r)),
                ">=" => return Some(ConstantValue::Bool(l >= r)),
                "==" => return Some(ConstantValue::Bool(l == r)),
                "!=" => return Some(ConstantValue::Bool(l != r)),
                _ => return None,
            };
            Some(ConstantValue::Int(result))
        }
        (ConstantValue::Bool(l), ConstantValue::Bool(r)) => {
            let result = match op {
                "and" | "&&" => *l && *r,
                "or" | "||" => *l || *r,
                "==" => l == r,
                "!=" => l != r,
                _ => return None,
            };
            Some(ConstantValue::Bool(result))
        }
        _ => None,
    }
}

/// Sparse Conditional Constant Propagation
///
/// # Algorithm (Wegman-Zadeck)
/// ```text
/// Initialize:
///   All SSA names = Top (unknown)
///   All blocks = unreachable
///   Worklist = {entry block}
///
/// Iterate:
///   While worklist not empty:
///     For executable blocks:
///       Evaluate instructions
///       Meet values: Top meet C = C, C meet C = C, else Bottom
///     For conditional branches:
///       Only add target if condition known true/false or unknown
/// ```
///
/// # Postconditions
/// - `constants` maps SSA names to their constant values
/// - `unreachable_blocks` contains provably dead code
/// - `dead_names` contains SSA names never used
pub fn run_sccp(ssa: &SsaFunction) -> TldrResult<SccpResult> {
    use super::types::SsaInstructionKind;
    use std::collections::VecDeque;

    // Initialize all SSA names to Top
    let mut lattice_values: HashMap<SsaNameId, LatticeValue> = HashMap::new();
    for name in &ssa.ssa_names {
        lattice_values.insert(name.id, LatticeValue::Top);
    }

    // Track executable blocks and edges
    let mut executable_blocks: HashSet<usize> = HashSet::new();
    let mut executable_edges: HashSet<(usize, usize)> = HashSet::new();

    // Two worklists: CFG edges and SSA edges
    let mut cfg_worklist: VecDeque<usize> = VecDeque::new();
    let mut ssa_worklist: VecDeque<SsaNameId> = VecDeque::new();

    // Find entry block
    let entry_block = ssa.blocks.first().map(|b| b.id).unwrap_or(0);
    cfg_worklist.push_back(entry_block);

    // Build block ID to block index map
    let block_index: HashMap<usize, usize> = ssa
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();

    // Build use map: SSA name -> instructions that use it
    let mut use_map: HashMap<SsaNameId, Vec<(usize, usize)>> = HashMap::new(); // name -> (block_id, instr_idx)
    for block in &ssa.blocks {
        for (instr_idx, instr) in block.instructions.iter().enumerate() {
            for &use_name in &instr.uses {
                use_map
                    .entry(use_name)
                    .or_default()
                    .push((block.id, instr_idx));
            }
        }
    }

    // Main SCCP loop
    let max_iterations = ssa.blocks.len() * ssa.ssa_names.len() + 100;
    let mut iterations = 0;

    while (!cfg_worklist.is_empty() || !ssa_worklist.is_empty()) && iterations < max_iterations {
        iterations += 1;

        // Process CFG worklist
        while let Some(block_id) = cfg_worklist.pop_front() {
            if !executable_blocks.insert(block_id) {
                continue; // Already processed
            }

            let Some(&block_idx) = block_index.get(&block_id) else {
                continue;
            };
            let block = &ssa.blocks[block_idx];

            // Process phi functions
            for phi in &block.phi_functions {
                // Evaluate phi considering only executable edges
                let mut result = LatticeValue::Top;
                for source in &phi.sources {
                    if executable_edges.contains(&(source.block, block_id))
                        || executable_blocks.contains(&source.block)
                    {
                        if let Some(source_val) = lattice_values.get(&source.name) {
                            result = lattice_meet(&result, source_val);
                        }
                    }
                }

                // Update if changed
                if let Some(current) = lattice_values.get(&phi.target) {
                    if *current != result {
                        lattice_values.insert(phi.target, result);
                        ssa_worklist.push_back(phi.target);
                    }
                }
            }

            // Process instructions and track if we have a branch
            let mut has_branch = false;

            for instr in &block.instructions {
                if let Some(target) = instr.target {
                    let new_value = evaluate_instruction(instr, &lattice_values);

                    if let Some(current) = lattice_values.get(&target) {
                        if *current != new_value {
                            lattice_values.insert(target, new_value);
                            ssa_worklist.push_back(target);
                        }
                    } else {
                        lattice_values.insert(target, new_value);
                        ssa_worklist.push_back(target);
                    }
                }

                // Handle conditional branches
                if instr.kind == SsaInstructionKind::Branch {
                    has_branch = true;

                    if !instr.uses.is_empty() {
                        let cond_name = instr.uses[0];
                        let cond_value = lattice_values.get(&cond_name);

                        match cond_value {
                            Some(LatticeValue::Constant(ConstantValue::Bool(true))) => {
                                // Only true branch is executable (first successor)
                                if let Some(&succ) = block.successors.first() {
                                    executable_edges.insert((block_id, succ));
                                    cfg_worklist.push_back(succ);
                                }
                            }
                            Some(LatticeValue::Constant(ConstantValue::Bool(false))) => {
                                // Only false branch is executable (second successor)
                                if let Some(&succ) = block.successors.get(1) {
                                    executable_edges.insert((block_id, succ));
                                    cfg_worklist.push_back(succ);
                                }
                            }
                            _ => {
                                // Unknown - both branches executable
                                for &succ in &block.successors {
                                    executable_edges.insert((block_id, succ));
                                    cfg_worklist.push_back(succ);
                                }
                            }
                        }
                    } else {
                        // Unconditional branch - all successors executable
                        for &succ in &block.successors {
                            executable_edges.insert((block_id, succ));
                            cfg_worklist.push_back(succ);
                        }
                    }
                }
            }

            // If no branch instruction, fall through to successors
            if !has_branch {
                for &succ in &block.successors {
                    if !executable_blocks.contains(&succ) {
                        executable_edges.insert((block_id, succ));
                        cfg_worklist.push_back(succ);
                    }
                }
            }
        }

        // Process SSA worklist - propagate constant values to uses
        while let Some(name) = ssa_worklist.pop_front() {
            if let Some(uses) = use_map.get(&name) {
                for &(block_id, _instr_idx) in uses {
                    if executable_blocks.contains(&block_id) {
                        // Re-evaluate this block's instructions
                        cfg_worklist.push_back(block_id);
                    }
                }
            }
        }
    }

    // Collect results
    let mut constants: HashMap<SsaNameId, ConstantValue> = HashMap::new();
    for (name, value) in &lattice_values {
        if let LatticeValue::Constant(c) = value {
            constants.insert(*name, c.clone());
        }
    }

    // Find unreachable blocks
    let all_blocks: HashSet<usize> = ssa.blocks.iter().map(|b| b.id).collect();
    let unreachable_blocks: HashSet<usize> = all_blocks
        .difference(&executable_blocks)
        .copied()
        .collect();

    // Find dead names (not used anywhere)
    let mut used_names: HashSet<SsaNameId> = HashSet::new();
    for block in &ssa.blocks {
        if executable_blocks.contains(&block.id) {
            for instr in &block.instructions {
                used_names.extend(instr.uses.iter().copied());
            }
            for phi in &block.phi_functions {
                for source in &phi.sources {
                    used_names.insert(source.name);
                }
            }
        }
    }

    let all_names: HashSet<SsaNameId> = ssa.ssa_names.iter().map(|n| n.id).collect();
    let dead_names: HashSet<SsaNameId> = all_names.difference(&used_names).copied().collect();

    Ok(SccpResult {
        function: ssa.function.clone(),
        constants,
        unreachable_blocks,
        dead_names,
    })
}

/// Evaluate an instruction to get its lattice value
fn evaluate_instruction(
    instr: &super::types::SsaInstruction,
    lattice_values: &HashMap<SsaNameId, LatticeValue>,
) -> LatticeValue {
    use super::types::SsaInstructionKind;

    match &instr.kind {
        SsaInstructionKind::Param => {
            // Parameters are not constant (unless we have interprocedural info)
            LatticeValue::Bottom
        }
        SsaInstructionKind::Assign => {
            if instr.uses.is_empty() {
                // Constant assignment
                if let Some(ref src) = instr.source_text {
                    if let Some(c) = parse_constant(src) {
                        return LatticeValue::Constant(c);
                    }
                }
                LatticeValue::Bottom
            } else if instr.uses.len() == 1 {
                // Copy: propagate source value
                lattice_values
                    .get(&instr.uses[0])
                    .cloned()
                    .unwrap_or(LatticeValue::Top)
            } else {
                LatticeValue::Bottom
            }
        }
        SsaInstructionKind::BinaryOp => {
            if instr.uses.len() >= 2 {
                let left = lattice_values
                    .get(&instr.uses[0])
                    .cloned()
                    .unwrap_or(LatticeValue::Top);
                let right = lattice_values
                    .get(&instr.uses[1])
                    .cloned()
                    .unwrap_or(LatticeValue::Top);

                match (&left, &right) {
                    (LatticeValue::Bottom, _) | (_, LatticeValue::Bottom) => LatticeValue::Bottom,
                    (LatticeValue::Top, _) | (_, LatticeValue::Top) => LatticeValue::Top,
                    (LatticeValue::Constant(l), LatticeValue::Constant(r)) => {
                        // Try to evaluate the operation
                        let op = instr.source_text.as_ref().and_then(|s| {
                            if s.contains('+') {
                                Some("+")
                            } else if s.contains('-') && !s.starts_with('-') {
                                Some("-")
                            } else if s.contains('*') {
                                Some("*")
                            } else if s.contains('/') {
                                Some("/")
                            } else if s.contains('<') && s.contains('=') {
                                Some("<=")
                            } else if s.contains('>') && s.contains('=') {
                                Some(">=")
                            } else if s.contains('<') {
                                Some("<")
                            } else if s.contains('>') {
                                Some(">")
                            } else if s.contains("==") {
                                Some("==")
                            } else if s.contains("!=") {
                                Some("!=")
                            } else {
                                None
                            }
                        });

                        if let Some(op) = op {
                            if let Some(result) = eval_binary_op(op, l, r) {
                                return LatticeValue::Constant(result);
                            }
                        }
                        LatticeValue::Bottom
                    }
                }
            } else {
                LatticeValue::Bottom
            }
        }
        SsaInstructionKind::UnaryOp => {
            if !instr.uses.is_empty() {
                let operand = lattice_values
                    .get(&instr.uses[0])
                    .cloned()
                    .unwrap_or(LatticeValue::Top);

                match operand {
                    LatticeValue::Bottom => LatticeValue::Bottom,
                    LatticeValue::Top => LatticeValue::Top,
                    LatticeValue::Constant(c) => {
                        // Try to evaluate unary ops
                        if let Some(ref src) = instr.source_text {
                            if src.contains("not") || src.contains('!') {
                                if let ConstantValue::Bool(b) = c {
                                    return LatticeValue::Constant(ConstantValue::Bool(!b));
                                }
                            }
                            if src.contains('-') {
                                if let ConstantValue::Int(i) = c {
                                    if let Some(neg) = i.checked_neg() {
                                        return LatticeValue::Constant(ConstantValue::Int(neg));
                                    }
                                }
                            }
                        }
                        LatticeValue::Bottom
                    }
                }
            } else {
                LatticeValue::Bottom
            }
        }
        SsaInstructionKind::Call => {
            // Calls are not constant (could have side effects)
            LatticeValue::Bottom
        }
        SsaInstructionKind::Return | SsaInstructionKind::Branch => {
            // These don't produce values
            LatticeValue::Bottom
        }
    }
}

// =============================================================================
// Dead Code Detection
// =============================================================================

/// Find dead code using SSA def-use information
///
/// # Algorithm
/// A definition is dead if:
/// 1. It has no uses (except if it has side effects)
/// 2. Iteratively remove until fixed point (cascading dead code)
///
/// # Returns
/// Vector of dead SSA names
pub fn find_dead_code(ssa: &SsaFunction) -> TldrResult<Vec<SsaNameId>> {
    // Build use count map: how many times each SSA name is used
    let mut use_count: HashMap<SsaNameId, usize> = HashMap::new();

    // Initialize all SSA names with 0 uses
    for name in &ssa.ssa_names {
        use_count.insert(name.id, 0);
    }

    // Count uses from instructions
    for block in &ssa.blocks {
        for instr in &block.instructions {
            for &use_name in &instr.uses {
                *use_count.entry(use_name).or_insert(0) += 1;
            }
        }
        // Count uses from phi functions
        for phi in &block.phi_functions {
            for source in &phi.sources {
                *use_count.entry(source.name).or_insert(0) += 1;
            }
        }
    }

    // Build def map: SSA name -> (block_id, instruction)
    let mut def_info: HashMap<SsaNameId, (usize, Option<&super::types::SsaInstruction>)> =
        HashMap::new();

    for block in &ssa.blocks {
        for instr in &block.instructions {
            if let Some(target) = instr.target {
                def_info.insert(target, (block.id, Some(instr)));
            }
        }
        for phi in &block.phi_functions {
            def_info.insert(phi.target, (block.id, None)); // Phi functions don't have side effects
        }
    }

    // Iteratively find dead code until fixed point
    let mut dead: HashSet<SsaNameId> = HashSet::new();
    let mut changed = true;
    let max_iterations = ssa.ssa_names.len() + 10;
    let mut iterations = 0;

    while changed && iterations < max_iterations {
        changed = false;
        iterations += 1;

        for name in &ssa.ssa_names {
            // Skip if already marked dead
            if dead.contains(&name.id) {
                continue;
            }

            // Get use count
            let uses = use_count.get(&name.id).copied().unwrap_or(0);

            // If no uses, check if it has side effects
            if uses == 0 {
                let has_effects = if let Some((_, Some(instr))) = def_info.get(&name.id) {
                    has_side_effects(&instr.kind)
                } else {
                    false // Phi functions have no side effects
                };

                if !has_effects {
                    // Mark as dead
                    dead.insert(name.id);
                    changed = true;

                    // Decrement use counts of this definition's operands
                    if let Some((block_id, _)) = def_info.get(&name.id) {
                        // Find the instruction/phi that defines this
                        for block in &ssa.blocks {
                            if block.id == *block_id {
                                // Check instructions
                                for instr in &block.instructions {
                                    if instr.target == Some(name.id) {
                                        for &use_name in &instr.uses {
                                            if let Some(count) = use_count.get_mut(&use_name) {
                                                *count = count.saturating_sub(1);
                                            }
                                        }
                                    }
                                }
                                // Check phi functions
                                for phi in &block.phi_functions {
                                    if phi.target == name.id {
                                        for source in &phi.sources {
                                            if let Some(count) = use_count.get_mut(&source.name) {
                                                *count = count.saturating_sub(1);
                                            }
                                        }
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // Convert to sorted vector for deterministic output
    let mut result: Vec<SsaNameId> = dead.into_iter().collect();
    result.sort_by_key(|id| id.0);

    Ok(result)
}

/// Check if an instruction has side effects
pub fn has_side_effects(kind: &super::types::SsaInstructionKind) -> bool {
    matches!(
        kind,
        super::types::SsaInstructionKind::Call | super::types::SsaInstructionKind::Return
    )
}

// =============================================================================
// SSA-Based Liveness
// =============================================================================

/// SSA liveness information for O(1) queries
#[derive(Debug, Clone)]
pub struct SsaLiveness {
    /// Function name
    pub function: String,
    /// Precomputed dominance intervals
    #[allow(dead_code)]
    dom_intervals: HashMap<usize, (u32, u32)>,
    /// Use positions for each SSA name
    #[allow(dead_code)]
    use_positions: HashMap<SsaNameId, Vec<(usize, u32)>>,
}

/// Position within a block
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockPosition {
    /// At block entry (before any instructions)
    Entry,
    /// After instruction at given index
    After(usize),
    /// At block exit (after all instructions)
    Exit,
}

/// Build SSA liveness for O(1) queries
///
/// # Algorithm (Boissinot et al.)
/// - Precompute dominator tree intervals
/// - Live = defined before point AND used after point in some path
/// - O(1) query after O(n) preprocessing
///
/// Simplified version: precompute live ranges for each SSA name
pub fn build_ssa_liveness(ssa: &SsaFunction) -> TldrResult<SsaLiveness> {
    // Build block ordering for comparison
    let mut block_order: HashMap<usize, u32> = HashMap::new();
    for (i, block) in ssa.blocks.iter().enumerate() {
        block_order.insert(block.id, i as u32);
    }

    // Compute dominance intervals (approximation using block order)
    let mut dom_intervals: HashMap<usize, (u32, u32)> = HashMap::new();
    for block in &ssa.blocks {
        let order = block_order.get(&block.id).copied().unwrap_or(0);
        // Simple interval: from this block to end
        // (A proper implementation would use dominator tree DFS numbering)
        dom_intervals.insert(block.id, (order, ssa.blocks.len() as u32));
    }

    // Collect use positions for each SSA name
    // (block_id, instruction_index)
    let mut use_positions: HashMap<SsaNameId, Vec<(usize, u32)>> = HashMap::new();

    for block in &ssa.blocks {
        for (instr_idx, instr) in block.instructions.iter().enumerate() {
            for &use_name in &instr.uses {
                use_positions
                    .entry(use_name)
                    .or_default()
                    .push((block.id, instr_idx as u32));
            }
        }
        // Uses in phi functions
        for phi in &block.phi_functions {
            for source in &phi.sources {
                // Phi uses are at block entry (position 0)
                use_positions
                    .entry(source.name)
                    .or_default()
                    .push((block.id, 0));
            }
        }
    }

    Ok(SsaLiveness {
        function: ssa.function.clone(),
        dom_intervals,
        use_positions,
    })
}

/// Query if SSA name is live at a point
///
/// An SSA name is live at a point if:
/// 1. It is defined before that point
/// 2. It has at least one use after that point (on some path)
pub fn is_live_at(
    liveness: &SsaLiveness,
    name: SsaNameId,
    block: usize,
    position: BlockPosition,
) -> bool {
    // Get use positions for this name
    let Some(uses) = liveness.use_positions.get(&name) else {
        return false; // No uses means not live
    };

    // Convert position to comparable value
    let query_pos = match position {
        BlockPosition::Entry => 0,
        BlockPosition::After(idx) => (idx + 1) as u32,
        BlockPosition::Exit => u32::MAX,
    };

    // Get block order for comparison
    let query_block_order = liveness
        .dom_intervals
        .get(&block)
        .map(|(start, _)| *start)
        .unwrap_or(0);

    // Check if any use is after the query point
    for &(use_block, use_pos) in uses {
        let use_block_order = liveness
            .dom_intervals
            .get(&use_block)
            .map(|(start, _)| *start)
            .unwrap_or(0);

        // Use is after query point if:
        // 1. Use is in a later block, OR
        // 2. Use is in same block but at a later position
        if use_block_order > query_block_order {
            return true;
        }
        if use_block == block && use_pos > query_pos {
            return true;
        }
    }

    false
}

// =============================================================================
// Available Expressions
// =============================================================================

/// Available expressions at each program point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableExpressions {
    /// Function name
    pub function: String,
    /// For each block, expressions available at entry and exit
    pub blocks: HashMap<usize, ExpressionSets>,
    /// All unique expressions
    pub expressions: Vec<Expression>,
}

/// Expression availability sets for a block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpressionSets {
    /// Expression indices available at block entry
    pub in_set: HashSet<usize>,
    /// Expression indices available at block exit
    pub out_set: HashSet<usize>,
}

/// An expression tracked for availability
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Expression {
    /// Expression text or normalized form
    pub text: String,
    /// Variables used in expression
    pub uses: Vec<String>,
    /// Line where first computed
    pub first_line: u32,
}

impl AvailableExpressions {
    /// Check if expression is available at start of block
    pub fn is_available(&self, block_id: usize, expr_text: &str) -> bool {
        // Find expression index
        let expr_idx = self.expressions.iter().position(|e| e.text == expr_text);
        if let Some(idx) = expr_idx {
            self.blocks
                .get(&block_id)
                .map(|sets| sets.in_set.contains(&idx))
                .unwrap_or(false)
        } else {
            false
        }
    }

    /// Get all available expressions at block entry
    pub fn available_at(&self, block_id: usize) -> Vec<&Expression> {
        self.blocks
            .get(&block_id)
            .map(|sets| {
                sets.in_set
                    .iter()
                    .filter_map(|&idx| self.expressions.get(idx))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl LiveVariables {
    /// Get all live variables at a specific line within a block
    pub fn live_at_line(&self, cfg: &CfgInfo, line: u32) -> HashSet<String> {
        // Find containing block
        let block = cfg
            .blocks
            .iter()
            .find(|b| b.lines.0 <= line && line <= b.lines.1);

        if let Some(block) = block {
            // Conservative: use live-in for the block
            self.blocks
                .get(&block.id)
                .map(|sets| sets.live_in.clone())
                .unwrap_or_default()
        } else {
            HashSet::new()
        }
    }

    /// Get live range for a variable (blocks where it's live)
    pub fn live_range(&self, var: &str, cfg: &CfgInfo) -> Vec<(u32, u32)> {
        let mut ranges = Vec::new();

        for block in &cfg.blocks {
            if self.is_live_in(block.id, var) || self.is_live_out(block.id, var) {
                ranges.push(block.lines);
            }
        }

        ranges
    }
}

/// Compute available expressions (forward must-analysis)
///
/// # Dataflow Equations
/// ```text
/// IN[B] = intersection OUT[P] for all predecessors P
/// OUT[B] = GEN[B] union (IN[B] - KILL[B])
///
/// GEN[B] = expressions computed in B (not killed later in B)
/// KILL[B] = expressions with operands modified in B
/// ```
///
/// # Algorithm
/// 1. Extract expressions from source using tree-sitter AST (with regex fallback)
/// 2. Compute GEN and KILL sets for each block
/// 3. Iterate forward dataflow until fixed point
///
/// The `language` parameter enables AST-based parsing for all 18 supported languages.
/// If tree-sitter parsing fails for a language, falls back to regex-based extraction.
pub fn compute_available_expressions(
    cfg: &CfgInfo,
    source: &str,
    language: Language,
) -> TldrResult<AvailableExpressions> {
    // Build predecessor map
    let mut predecessors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        predecessors.insert(block.id, Vec::new());
    }
    for edge in &cfg.edges {
        predecessors.entry(edge.to).or_default().push(edge.from);
    }

    // Build line-to-block mapping
    let line_to_block: HashMap<u32, usize> = cfg
        .blocks
        .iter()
        .flat_map(|block| (block.lines.0..=block.lines.1).map(move |line| (line, block.id)))
        .collect();

    // Extract expressions and definitions using AST (with regex fallback)
    let extraction = extract_expressions_and_defs(source, language, &line_to_block, cfg);

    let all_expressions = extraction.all_expressions;
    let expr_to_index = extraction.expr_to_index;
    let block_defs = extraction.block_defs;
    let block_exprs = extraction.block_exprs;
    let defs_by_line = extraction.defs_by_line;

    // If no expressions found, return empty result
    if all_expressions.is_empty() {
        let mut blocks_result = HashMap::new();
        for block in &cfg.blocks {
            blocks_result.insert(
                block.id,
                ExpressionSets {
                    in_set: HashSet::new(),
                    out_set: HashSet::new(),
                },
            );
        }
        return Ok(AvailableExpressions {
            function: cfg.function.clone(),
            blocks: blocks_result,
            expressions: all_expressions,
        });
    }

    // Compute GEN and KILL sets for each block
    let mut gen_sets: HashMap<usize, HashSet<usize>> = HashMap::new();
    let mut kill_sets: HashMap<usize, HashSet<usize>> = HashMap::new();

    for block in &cfg.blocks {
        let mut gen = HashSet::new();
        let mut kill = HashSet::new();

        // KILL: expressions whose operands are modified in this block
        let defs = block_defs.get(&block.id).cloned().unwrap_or_default();
        for (idx, expr) in all_expressions.iter().enumerate() {
            if expr.uses.iter().any(|v| defs.contains(v)) {
                kill.insert(idx);
            }
        }

        // GEN: expressions computed in this block (that aren't killed later in block)
        if let Some(exprs) = block_exprs.get(&block.id) {
            for (line_num, expr) in exprs {
                if let Some(&idx) = expr_to_index.get(&expr.text) {
                    // Check if any operand is redefined after this expression in the same block
                    let killed_after = defs_by_line.iter().any(|(&def_line, def_var)| {
                        def_line > *line_num
                            && def_line <= block.lines.1
                            && expr.uses.contains(def_var)
                    });

                    if !killed_after {
                        gen.insert(idx);
                    }
                }
            }
        }

        gen_sets.insert(block.id, gen);
        kill_sets.insert(block.id, kill);
    }

    // Initialize IN and OUT sets
    // IN[entry] = empty
    // IN[other] = U (all expressions) for must-analysis
    // OUT[B] = GEN[B] for initialization
    let all_expr_indices: HashSet<usize> = (0..all_expressions.len()).collect();

    let mut avail_in: HashMap<usize, HashSet<usize>> = HashMap::new();
    let mut avail_out: HashMap<usize, HashSet<usize>> = HashMap::new();

    for block in &cfg.blocks {
        if block.id == cfg.entry_block {
            avail_in.insert(block.id, HashSet::new());
        } else {
            avail_in.insert(block.id, all_expr_indices.clone());
        }
        avail_out.insert(
            block.id,
            gen_sets.get(&block.id).cloned().unwrap_or_default(),
        );
    }

    // Forward iteration until fixed point
    let block_ids: Vec<usize> = cfg.blocks.iter().map(|b| b.id).collect();
    let max_iterations = block_ids.len() * 2 + 10;
    let mut changed = true;
    let mut _iterations = 0;

    while changed && _iterations < max_iterations {
        changed = false;
        _iterations += 1;

        for &block_id in &block_ids {
            // IN[B] = intersection of OUT[P] for all predecessors P
            let preds = predecessors.get(&block_id).cloned().unwrap_or_default();
            let new_in = if preds.is_empty() {
                HashSet::new()
            } else {
                let mut intersection = all_expr_indices.clone();
                for pred in &preds {
                    if let Some(pred_out) = avail_out.get(pred) {
                        intersection = intersection.intersection(pred_out).copied().collect();
                    }
                }
                intersection
            };

            // OUT[B] = GEN[B] union (IN[B] - KILL[B])
            let gen = gen_sets.get(&block_id).cloned().unwrap_or_default();
            let kill = kill_sets.get(&block_id).cloned().unwrap_or_default();
            let in_minus_kill: HashSet<usize> = new_in.difference(&kill).copied().collect();
            let new_out: HashSet<usize> = gen.union(&in_minus_kill).copied().collect();

            // Check for changes
            if new_in != *avail_in.get(&block_id).unwrap_or(&HashSet::new()) {
                changed = true;
                avail_in.insert(block_id, new_in);
            }
            if new_out != *avail_out.get(&block_id).unwrap_or(&HashSet::new()) {
                changed = true;
                avail_out.insert(block_id, new_out);
            }
        }
    }

    // Build result
    let mut blocks_result = HashMap::new();
    for block in &cfg.blocks {
        blocks_result.insert(
            block.id,
            ExpressionSets {
                in_set: avail_in.get(&block.id).cloned().unwrap_or_default(),
                out_set: avail_out.get(&block.id).cloned().unwrap_or_default(),
            },
        );
    }

    Ok(AvailableExpressions {
        function: cfg.function.clone(),
        blocks: blocks_result,
        expressions: all_expressions,
    })
}

// =============================================================================
// Expression Extraction (AST-based with regex fallback)
// =============================================================================

/// Intermediate result from expression extraction
struct ExpressionExtraction {
    all_expressions: Vec<Expression>,
    expr_to_index: HashMap<String, usize>,
    block_defs: HashMap<usize, HashSet<String>>,
    block_exprs: HashMap<usize, Vec<(u32, Expression)>>,
    defs_by_line: HashMap<u32, String>,
}

/// Extract expressions and definitions from source code.
///
/// Attempts AST-based extraction using tree-sitter for the given language.
/// Falls back to regex-based extraction if AST parsing fails.
fn extract_expressions_and_defs(
    source: &str,
    language: Language,
    line_to_block: &HashMap<u32, usize>,
    cfg: &CfgInfo,
) -> ExpressionExtraction {
    // Try AST-based extraction first
    if let Some(result) = extract_expressions_ast(source, language, line_to_block, cfg) {
        return result;
    }

    // Fallback to regex
    extract_expressions_regex(source, line_to_block, cfg)
}

/// AST-based expression extraction using tree-sitter.
///
/// Uses a two-pass approach:
/// 1. Walk all nodes to find assignments and record definitions (LHS variables)
/// 2. Walk all nodes to find binary expressions inside assignments and record them
///
/// This approach is more robust than trying to match assignment types and extract
/// RHS per language -- instead, we find binary expressions directly and walk UP
/// to find their parent assignment.
fn extract_expressions_ast(
    source: &str,
    language: Language,
    line_to_block: &HashMap<u32, usize>,
    cfg: &CfgInfo,
) -> Option<ExpressionExtraction> {
    use crate::ast::parser::ParserPool;
    use crate::security::ast_utils::{
        binary_expression_node_kinds, walk_descendants,
    };

    let pool = ParserPool::new();
    let tree = pool.parse(source, language).ok()?;
    let src_bytes = source.as_bytes();

    let binop_kinds = binary_expression_node_kinds(language);

    let mut all_expressions: Vec<Expression> = Vec::new();
    let mut expr_to_index: HashMap<String, usize> = HashMap::new();
    let mut defs_by_line: HashMap<u32, String> = HashMap::new();

    let mut block_defs: HashMap<usize, HashSet<String>> = HashMap::new();
    for block in &cfg.blocks {
        block_defs.insert(block.id, HashSet::new());
    }

    let mut block_exprs: HashMap<usize, Vec<(u32, Expression)>> = HashMap::new();
    for block in &cfg.blocks {
        block_exprs.insert(block.id, Vec::new());
    }

    let descendants = walk_descendants(tree.root_node());

    // Pass 1: Find all assignments and record definitions.
    // Use a broad set of assignment-like node kinds per language.
    for node in &descendants {
        if let Some(var_name) = extract_def_from_node(node, src_bytes, language) {
            let line_num = node.start_position().row as u32 + 1;
            if let Some(&block_id) = line_to_block.get(&line_num) {
                block_defs.entry(block_id).or_default().insert(var_name.clone());
                defs_by_line.insert(line_num, var_name);
            }
        }
    }

    // Pass 2: Find all binary expressions that are arithmetic operators
    // (not assignments like `=` or comparisons like `==`).
    for node in &descendants {
        if !binop_kinds.contains(&node.kind()) {
            continue;
        }

        if let Some((left, op, right)) = extract_binop_operands(node, src_bytes, language) {
            // Only track arithmetic operators for CSE
            if !is_arithmetic_op(&op) {
                continue;
            }

            // Check that operands are simple identifiers (not complex expressions)
            if !is_simple_identifier(&left) || !is_simple_identifier(&right) {
                continue;
            }

            let line_num = node.start_position().row as u32 + 1;

            // Canonicalize: sort operands for commutative ops
            let (canonical_left, canonical_right) = if op == "+" || op == "*" {
                if left < right {
                    (left, right)
                } else {
                    (right, left)
                }
            } else {
                (left, right)
            };

            let expr_text = format!("{} {} {}", canonical_left, op, canonical_right);

            let expr_idx = if let Some(&idx) = expr_to_index.get(&expr_text) {
                idx
            } else {
                let idx = all_expressions.len();
                let expr = Expression {
                    text: expr_text.clone(),
                    uses: vec![canonical_left.clone(), canonical_right.clone()],
                    first_line: line_num,
                };
                all_expressions.push(expr.clone());
                expr_to_index.insert(expr_text, idx);
                idx
            };

            if let Some(&block_id) = line_to_block.get(&line_num) {
                let expr = all_expressions[expr_idx].clone();
                block_exprs.entry(block_id).or_default().push((line_num, expr));
            }
        }
    }

    Some(ExpressionExtraction {
        all_expressions,
        expr_to_index,
        block_defs,
        block_exprs,
        defs_by_line,
    })
}

/// Check if an operator is an arithmetic operator (for CSE tracking).
fn is_arithmetic_op(op: &str) -> bool {
    matches!(op, "+" | "-" | "*" | "/" | "%" | "**" | "//" | "<<" | ">>" | "&" | "|" | "^")
}

/// Check if a string is a simple identifier (variable name).
///
/// Rejects complex expressions like function calls, member access, etc.
fn is_simple_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let s = s.trim_start_matches('$'); // PHP variables
    if s.is_empty() {
        return false;
    }
    let first = s.chars().next().unwrap();
    if !first.is_alphabetic() && first != '_' {
        return false;
    }
    s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// Extract a definition (variable name) from any node that defines a variable.
///
/// This is broader than just looking at assignment_node_kinds -- it handles
/// language-specific node structures that define variables.
fn extract_def_from_node(
    node: &tree_sitter::Node,
    source: &[u8],
    language: Language,
) -> Option<String> {
    use crate::security::ast_utils::{assignment_node_kinds, node_text};

    let assign_kinds = assignment_node_kinds(language);
    let kind = node.kind();

    // Check standard assignment node kinds first
    if assign_kinds.contains(&kind) {
        return extract_lhs_from_assignment(node, source, language);
    }

    // Additional assignment-like node kinds not covered by the standard list
    match language {
        Language::TypeScript | Language::JavaScript => {
            // lexical_declaration (const/let) wraps variable_declarator
            if kind == "lexical_declaration" {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "variable_declarator" {
                            if let Some(name) = child.child_by_field_name("name") {
                                return Some(node_text(&name, source).to_string());
                            }
                        }
                    }
                }
            }
        }
        Language::Elixir => {
            // In Elixir, `x = expr` is a binary_operator with `=` operator
            if kind == "binary_operator" {
                if let Some(op) = node.child_by_field_name("operator") {
                    if node_text(&op, source) == "=" {
                        if let Some(left) = node.child_by_field_name("left") {
                            let text = node_text(&left, source).to_string();
                            if is_simple_identifier(&text) {
                                return Some(text);
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    None
}

/// Extract the LHS variable name from a known assignment node.
fn extract_lhs_from_assignment(
    node: &tree_sitter::Node,
    source: &[u8],
    language: Language,
) -> Option<String> {
    use crate::security::ast_utils::node_text;

    // Try common field names first
    if let Some(left) = node.child_by_field_name("left") {
        let text = node_text(&left, source);
        // Handle comma-separated (Go: a, b := ...)
        return text.split(',').next().map(|s| s.trim().to_string());
    }

    // Try pattern field (Rust let, Scala val, OCaml let)
    if let Some(pattern) = node.child_by_field_name("pattern") {
        return Some(node_text(&pattern, source).to_string());
    }

    extract_lhs_language_fallback(node, source, language)
}

fn extract_lhs_language_fallback(
    node: &tree_sitter::Node,
    source: &[u8],
    language: Language,
) -> Option<String> {
    match language {
        Language::TypeScript | Language::JavaScript => extract_name_from_variable_declarator(node, source),
        Language::Java => {
            if node.kind() == "local_variable_declaration" {
                extract_name_from_variable_declarator(node, source)
            } else {
                None
            }
        }
        Language::Kotlin => extract_lhs_kotlin(node, source),
        Language::C | Language::Cpp => extract_lhs_c_family(node, source),
        Language::Rust => extract_lhs_rust(node, source),
        Language::CSharp => extract_lhs_csharp(node, source),
        Language::Swift => extract_lhs_swift(node, source),
        Language::Lua | Language::Luau => extract_lhs_lua(node, source, language),
        Language::Scala => None,
        Language::Elixir | Language::Ocaml => extract_first_child_text(node, source),
        _ => None,
    }
}

fn extract_name_from_variable_declarator(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<String> {
    use crate::security::ast_utils::node_text;

    for i in 0..node.child_count() {
        let child = node.child(i)?;
        if child.kind() != "variable_declarator" {
            continue;
        }
        if let Some(name) = child.child_by_field_name("name") {
            return Some(node_text(&name, source).to_string());
        }
    }
    None
}

fn extract_lhs_kotlin(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    for i in 0..node.child_count() {
        let child = node.child(i)?;
        if child.kind() == "variable_declaration" || child.kind() == "simple_identifier" {
            return Some(node_text(&child, source).to_string());
        }
    }
    None
}

fn extract_lhs_c_family(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    if node.kind() != "declaration" {
        return None;
    }
    for i in 0..node.child_count() {
        let child = node.child(i)?;
        if child.kind() != "init_declarator" {
            continue;
        }
        if let Some(decl) = child.child_by_field_name("declarator") {
            let text = node_text(&decl, source);
            return Some(text.trim_start_matches('*').to_string());
        }
    }
    None
}

fn extract_lhs_rust(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    if node.kind() != "let_declaration" {
        return None;
    }
    let pattern = node.child_by_field_name("pattern")?;
    Some(node_text(&pattern, source).to_string())
}

fn extract_lhs_csharp(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    if node.kind() != "variable_declaration" {
        return None;
    }
    for i in 0..node.child_count() {
        let child = node.child(i)?;
        if child.kind() != "variable_declarator" {
            continue;
        }
        if let Some(name) = child.child_by_field_name("name") {
            return Some(node_text(&name, source).to_string());
        }
        for j in 0..child.child_count() {
            let grandchild = child.child(j)?;
            if grandchild.kind() == "identifier" {
                return Some(node_text(&grandchild, source).to_string());
            }
        }
    }
    None
}

fn extract_lhs_swift(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    if node.kind() != "property_declaration" {
        return None;
    }
    for i in 0..node.child_count() {
        let child = node.child(i)?;
        if child.kind() == "pattern" {
            return Some(node_text(&child, source).to_string());
        }
    }
    None
}

fn extract_lhs_lua(
    node: &tree_sitter::Node,
    source: &[u8],
    language: Language,
) -> Option<String> {
    use crate::security::ast_utils::node_text;

    if node.kind() == "variable_declaration" {
        for i in 0..node.child_count() {
            let child = node.child(i)?;
            if child.kind() == "assignment_statement" {
                return extract_lhs_from_assignment(&child, source, language);
            }
        }
    }

    for i in 0..node.child_count() {
        let child = node.child(i)?;
        if child.kind() == "variable_list" || child.kind() == "assignment_variable_list" {
            if let Some(first) = child.child(0) {
                return Some(node_text(&first, source).to_string());
            }
        }
    }

    let first = node.child(0)?;
    if first.is_named() && first.kind() != "local" {
        return Some(node_text(&first, source).to_string());
    }
    None
}

fn extract_first_child_text(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    Some(node_text(&node.child(0)?, source).to_string())
}

/// Extract operands and operator from a binary expression node.
///
/// Returns (left_operand, operator, right_operand) as strings.
/// Extract operands and operator from a binary expression node.
///
/// Returns (left_operand, operator, right_operand) as strings.
/// Uses field-name access first, then falls back to positional access.
fn extract_binop_operands(
    node: &tree_sitter::Node,
    source: &[u8],
    _language: Language,
) -> Option<(String, String, String)> {
    use crate::security::ast_utils::node_text;

    // Strategy 1: Try field-name access (works for most languages)
    if let (Some(left), Some(right), Some(op)) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
        node.child_by_field_name("operator"),
    ) {
        return Some((
            node_text(&left, source).trim().to_string(),
            node_text(&op, source).trim().to_string(),
            node_text(&right, source).trim().to_string(),
        ));
    }

    // Strategy 2: Positional access -- binary nodes are typically [left, op, right]
    // This handles Kotlin (additive_expression, binary_expression), Scala (infix_expression),
    // OCaml (infix_expression), and languages where operator has no field name.
    if node.child_count() >= 3 {
        let left = node.child(0)?;
        let op = node.child(1)?;
        let right = node.child(2)?;

        // Validate: left and right should be named nodes, op should be an operator
        let op_text = node_text(&op, source).trim().to_string();
        if !op_text.is_empty() {
            return Some((
                node_text(&left, source).trim().to_string(),
                op_text,
                node_text(&right, source).trim().to_string(),
            ));
        }
    }

    // Strategy 3: Try left/right fields only, extract operator from text
    if let (Some(left), Some(right)) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
    ) {
        // The operator is between left and right in the source text
        // Try to find an unnamed child that's an operator
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if !child.is_named() {
                    let text = node_text(&child, source).trim().to_string();
                    if is_arithmetic_op(&text) {
                        return Some((
                            node_text(&left, source).trim().to_string(),
                            text,
                            node_text(&right, source).trim().to_string(),
                        ));
                    }
                }
            }
        }
    }

    None
}

/// Regex-based expression extraction (fallback when AST parsing fails).
///
/// Uses simple regex patterns to find assignments with binary expressions.
/// Only works reliably for Python-like syntax.
fn extract_expressions_regex(
    source: &str,
    line_to_block: &HashMap<u32, usize>,
    cfg: &CfgInfo,
) -> ExpressionExtraction {
    use regex::Regex;

    let assign_re = Regex::new(r"^\s*(\w+)\s*=\s*(.+)$").unwrap();
    let binop_re = Regex::new(r"(\w+)\s*([+\-*/])\s*(\w+)").unwrap();

    let lines: Vec<&str> = source.lines().collect();
    let mut all_expressions: Vec<Expression> = Vec::new();
    let mut expr_to_index: HashMap<String, usize> = HashMap::new();
    let mut defs_by_line: HashMap<u32, String> = HashMap::new();

    let mut block_defs: HashMap<usize, HashSet<String>> = HashMap::new();
    for block in &cfg.blocks {
        block_defs.insert(block.id, HashSet::new());
    }

    let mut block_exprs: HashMap<usize, Vec<(u32, Expression)>> = HashMap::new();
    for block in &cfg.blocks {
        block_exprs.insert(block.id, Vec::new());
    }

    for (line_idx, line) in lines.iter().enumerate() {
        let line_num = line_idx as u32 + 1;

        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(caps) = assign_re.captures(trimmed) {
            let var_name = caps.get(1).unwrap().as_str().to_string();
            let rhs = caps.get(2).unwrap().as_str();

            if let Some(&block_id) = line_to_block.get(&line_num) {
                block_defs.entry(block_id).or_default().insert(var_name.clone());
                defs_by_line.insert(line_num, var_name);
            }

            if let Some(binop_caps) = binop_re.captures(rhs) {
                let left = binop_caps.get(1).unwrap().as_str().to_string();
                let op = binop_caps.get(2).unwrap().as_str().to_string();
                let right = binop_caps.get(3).unwrap().as_str().to_string();

                let (canonical_left, canonical_right) = if op == "+" || op == "*" {
                    if left < right {
                        (left, right)
                    } else {
                        (right, left)
                    }
                } else {
                    (left, right)
                };

                let expr_text = format!("{} {} {}", canonical_left, op, canonical_right);

                let expr_idx = if let Some(&idx) = expr_to_index.get(&expr_text) {
                    idx
                } else {
                    let idx = all_expressions.len();
                    let expr = Expression {
                        text: expr_text.clone(),
                        uses: vec![canonical_left.clone(), canonical_right.clone()],
                        first_line: line_num,
                    };
                    all_expressions.push(expr.clone());
                    expr_to_index.insert(expr_text, idx);
                    idx
                };

                if let Some(&block_id) = line_to_block.get(&line_num) {
                    let expr = all_expressions[expr_idx].clone();
                    block_exprs.entry(block_id).or_default().push((line_num, expr));
                }
            }
        }
    }

    ExpressionExtraction {
        all_expressions,
        expr_to_index,
        block_defs,
        block_exprs,
        defs_by_line,
    }
}

/// Compute available expressions from VarRefs (forward must-analysis)
///
/// Alternative implementation that takes pre-extracted VarRefs instead of source code.
/// Useful when VarRefs have already been extracted from AST.
pub fn compute_available_expressions_from_refs(
    cfg: &CfgInfo,
    refs: &[crate::types::VarRef],
) -> TldrResult<AvailableExpressions> {
    use crate::types::RefType;

    // Build predecessor map
    let mut predecessors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        predecessors.insert(block.id, Vec::new());
    }
    for edge in &cfg.edges {
        predecessors.entry(edge.to).or_default().push(edge.from);
    }

    // Build line-to-block mapping
    let line_to_block: HashMap<u32, usize> = cfg
        .blocks
        .iter()
        .flat_map(|block| (block.lines.0..=block.lines.1).map(move |line| (line, block.id)))
        .collect();

    // Extract all expressions and variables defined in each block
    // For now, we track variables with uses (since we don't have full expression parsing)
    // An "expression" is represented by its operands
    let mut all_expressions: Vec<Expression> = Vec::new();
    let mut expr_to_index: HashMap<String, usize> = HashMap::new();

    // Track definitions in each block
    let mut block_defs: HashMap<usize, HashSet<String>> = HashMap::new();
    for block in &cfg.blocks {
        block_defs.insert(block.id, HashSet::new());
    }

    for var_ref in refs {
        if let Some(&block_id) = line_to_block.get(&var_ref.line) {
            if matches!(var_ref.ref_type, RefType::Definition | RefType::Update) {
                block_defs.entry(block_id).or_default().insert(var_ref.name.clone());
            }
        }
    }

    // Create expressions from uses that follow definitions
    // This is a simplified model where we track "x op y" style expressions
    // In a full implementation, we'd parse the source code for actual expressions
    let mut block_uses: HashMap<usize, Vec<&crate::types::VarRef>> = HashMap::new();
    for var_ref in refs {
        if matches!(var_ref.ref_type, RefType::Use) {
            if let Some(&block_id) = line_to_block.get(&var_ref.line) {
                block_uses.entry(block_id).or_default().push(var_ref);
            }
        }
    }

    // For a simplified model: create an expression for each pair of uses on the same line
    // This represents binary operations like "a + b"
    for (&_block_id, uses) in &block_uses {
        // Group uses by line
        let mut uses_by_line: HashMap<u32, Vec<&crate::types::VarRef>> = HashMap::new();
        for &var_ref in uses {
            uses_by_line.entry(var_ref.line).or_default().push(var_ref);
        }

        for (line, line_uses) in uses_by_line {
            if line_uses.len() >= 2 {
                // Create an expression from the first two uses
                let mut operands: Vec<String> = line_uses.iter().map(|r| r.name.clone()).collect();
                operands.sort(); // Canonicalize for commutative operations
                let expr_text = operands.join(" op ");

                if !expr_to_index.contains_key(&expr_text) {
                    let idx = all_expressions.len();
                    expr_to_index.insert(expr_text.clone(), idx);
                    all_expressions.push(Expression {
                        text: expr_text,
                        uses: operands,
                        first_line: line,
                    });
                }
            }
        }

        // Also track single-variable expressions for simple assignments
        for &var_ref in uses {
            let expr_text = format!("use_{}", var_ref.name);
            if !expr_to_index.contains_key(&expr_text) {
                let idx = all_expressions.len();
                expr_to_index.insert(expr_text.clone(), idx);
                all_expressions.push(Expression {
                    text: expr_text,
                    uses: vec![var_ref.name.clone()],
                    first_line: var_ref.line,
                });
            }
        }
    }

    // If no expressions found, return empty result
    if all_expressions.is_empty() {
        let mut blocks_result = HashMap::new();
        for block in &cfg.blocks {
            blocks_result.insert(
                block.id,
                ExpressionSets {
                    in_set: HashSet::new(),
                    out_set: HashSet::new(),
                },
            );
        }
        return Ok(AvailableExpressions {
            function: cfg.function.clone(),
            blocks: blocks_result,
            expressions: all_expressions,
        });
    }

    // Compute GEN and KILL sets for each block
    let mut gen_sets: HashMap<usize, HashSet<usize>> = HashMap::new();
    let mut kill_sets: HashMap<usize, HashSet<usize>> = HashMap::new();

    for block in &cfg.blocks {
        let mut gen = HashSet::new();
        let mut kill = HashSet::new();

        // KILL: expressions whose operands are modified in this block
        let defs = block_defs.get(&block.id).cloned().unwrap_or_default();
        for (idx, expr) in all_expressions.iter().enumerate() {
            if expr.uses.iter().any(|v| defs.contains(v)) {
                kill.insert(idx);
            }
        }

        // GEN: expressions computed in this block (that aren't killed later in block)
        // Find expressions whose first computation is in this block
        for (idx, expr) in all_expressions.iter().enumerate() {
            if expr.first_line >= block.lines.0 && expr.first_line <= block.lines.1 {
                // Check if any operand is redefined after this expression in the same block
                let mut killed_after = false;
                for var_ref in refs {
                    if var_ref.line > expr.first_line
                        && var_ref.line <= block.lines.1
                        && matches!(var_ref.ref_type, RefType::Definition | RefType::Update)
                        && expr.uses.contains(&var_ref.name)
                    {
                        killed_after = true;
                        break;
                    }
                }
                if !killed_after {
                    gen.insert(idx);
                }
            }
        }

        gen_sets.insert(block.id, gen);
        kill_sets.insert(block.id, kill);
    }

    // Initialize IN and OUT sets
    // IN[entry] = empty
    // IN[other] = U (all expressions) for must-analysis
    // OUT[B] = GEN[B] for initialization
    let all_expr_indices: HashSet<usize> = (0..all_expressions.len()).collect();

    let mut avail_in: HashMap<usize, HashSet<usize>> = HashMap::new();
    let mut avail_out: HashMap<usize, HashSet<usize>> = HashMap::new();

    for block in &cfg.blocks {
        if block.id == cfg.entry_block {
            avail_in.insert(block.id, HashSet::new());
        } else {
            avail_in.insert(block.id, all_expr_indices.clone());
        }
        avail_out.insert(
            block.id,
            gen_sets.get(&block.id).cloned().unwrap_or_default(),
        );
    }

    // Forward iteration until fixed point
    let block_ids: Vec<usize> = cfg.blocks.iter().map(|b| b.id).collect();
    let max_iterations = block_ids.len() * 2 + 10;
    let mut changed = true;
    let mut iterations = 0;

    while changed && iterations < max_iterations {
        changed = false;
        iterations += 1;

        for &block_id in &block_ids {
            // IN[B] = intersection of OUT[P] for all predecessors P
            let preds = predecessors.get(&block_id).cloned().unwrap_or_default();
            let new_in = if preds.is_empty() {
                HashSet::new()
            } else {
                let mut intersection = all_expr_indices.clone();
                for pred in &preds {
                    if let Some(pred_out) = avail_out.get(pred) {
                        intersection = intersection.intersection(pred_out).copied().collect();
                    }
                }
                intersection
            };

            // OUT[B] = GEN[B] union (IN[B] - KILL[B])
            let gen = gen_sets.get(&block_id).cloned().unwrap_or_default();
            let kill = kill_sets.get(&block_id).cloned().unwrap_or_default();
            let in_minus_kill: HashSet<usize> = new_in.difference(&kill).copied().collect();
            let new_out: HashSet<usize> = gen.union(&in_minus_kill).copied().collect();

            // Check for changes
            if new_in != *avail_in.get(&block_id).unwrap_or(&HashSet::new()) {
                changed = true;
                avail_in.insert(block_id, new_in);
            }
            if new_out != *avail_out.get(&block_id).unwrap_or(&HashSet::new()) {
                changed = true;
                avail_out.insert(block_id, new_out);
            }
        }
    }

    // Build result
    let mut blocks_result = HashMap::new();
    for block in &cfg.blocks {
        blocks_result.insert(
            block.id,
            ExpressionSets {
                in_set: avail_in.get(&block.id).cloned().unwrap_or_default(),
                out_set: avail_out.get(&block.id).cloned().unwrap_or_default(),
            },
        );
    }

    Ok(AvailableExpressions {
        function: cfg.function.clone(),
        blocks: blocks_result,
        expressions: all_expressions,
    })
}
