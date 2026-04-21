# Dataflow Analysis Specification

**Created**: 2026-02-03
**Author**: architect-agent
**Source**: Gap Analysis session13-available-abstract-gap.yaml
**Total Capabilities**: 34 (12 Available Expressions + 22 Abstract Interpretation)

## Overview

This specification defines the Rust port of two dataflow analyses from tldr-code v2:

1. **Available Expressions Analysis**: Forward MUST (intersection) dataflow analysis for CSE detection
2. **Abstract Interpretation**: Forward dataflow with widening for range tracking and safety checks

Both analyses build on the existing `tldr-core` infrastructure:
- `cfg` module for control flow graphs
- `dfg` module for variable references (VarRef)
- `ssa` module for dominance and def-use chains

## Module Structure

```
src/dataflow/
├── mod.rs              # Module entry point, re-exports
├── types.rs            # Shared types (BlockId, predecessors helper)
├── available.rs        # Available expressions analysis (CAP-AE-01 through CAP-AE-12)
├── abstract_interp.rs  # Abstract interpretation (CAP-AI-01 through CAP-AI-22)
└── tests/
    ├── available_test.rs
    └── abstract_interp_test.rs
```

---

## Part 1: Available Expressions Analysis

### Data Types

#### CAP-AE-01: Expression (frozen/hashable)

```rust
/// Represents a computed expression for availability analysis.
/// 
/// Immutable and hashable by text only (line-independent equality).
/// Two expressions with the same normalized text are equal regardless of line.
#[derive(Debug, Clone, Eq, Serialize, Deserialize)]
pub struct Expression {
    /// Normalized expression string (e.g., "a + b")
    pub text: String,
    
    /// Variables used in this expression (sorted for consistency)
    pub operands: Vec<String>,
    
    /// Line where expression first appears
    pub line: u32,
}

impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text  // Equality by text only
    }
}

impl Hash for Expression {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);  // Hash by text only
    }
}

impl Expression {
    /// Check if redefining var kills this expression.
    /// 
    /// An expression is killed when any operand is redefined.
    pub fn is_killed_by(&self, var: &str) -> bool {
        self.operands.iter().any(|op| op == var)
    }
}
```

#### CAP-AE-02: Commutative Expression Normalization

```rust
/// Commutative operators that allow operand reordering.
pub const COMMUTATIVE_OPS: &[&str] = &["+", "*", "==", "!=", "and", "or", "&", "|", "^"];

/// Normalize binary expression to canonical form.
/// 
/// For commutative operators, sort operands alphabetically.
/// This ensures "a + b" and "b + a" produce the same text.
pub fn normalize_expression(op: &str, left: &str, right: &str) -> String {
    if COMMUTATIVE_OPS.contains(&op) {
        let mut operands = [left.trim(), right.trim()];
        operands.sort();
        format!("{} {} {}", operands[0], op, operands[1])
    } else {
        format!("{} {} {}", left.trim(), op, right.trim())
    }
}
```

#### CAP-AE-03 to CAP-AE-07: BlockExpressions (Gen/Kill)

```rust
/// Expressions generated and killed in a single CFG block.
#[derive(Debug, Default)]
struct BlockExpressions {
    /// Expressions computed in this block (before any operand is killed)
    gen: HashSet<Expression>,
    
    /// Variables redefined in this block (kills expressions using that var)
    kill: HashSet<String>,
}
```

#### CAP-AE-08 to CAP-AE-11: AvailableExprsInfo

```rust
/// Available expressions analysis results.
/// 
/// An expression is available at a point if:
/// 1. It has been computed on EVERY path reaching that point (MUST analysis)
/// 2. None of its operands have been redefined since computation
#[derive(Debug, Serialize, Deserialize)]
pub struct AvailableExprsInfo {
    /// Expressions available at block entry.
    /// avail_in[b] = intersection(avail_out[p] for p in predecessors[b])
    pub avail_in: HashMap<u32, HashSet<Expression>>,
    
    /// Expressions available at block exit.
    /// avail_out[b] = gen[b] | (avail_in[b] - killed_by_block[b])
    pub avail_out: HashMap<u32, HashSet<Expression>>,
    
    /// All unique expressions found in the function
    pub all_exprs: HashSet<Expression>,
    
    /// Entry block ID
    pub entry_block: u32,
    
    /// All expression instances including duplicates (for CSE detection)
    #[serde(skip)]
    pub expr_instances: Vec<Expression>,
}

impl AvailableExprsInfo {
    /// Check if expression is available at entry to block.
    pub fn is_available(&self, block: u32, expr: &Expression) -> bool {
        self.avail_in.get(&block).map_or(false, |set| set.contains(expr))
    }
    
    /// Check if expression is available at exit of block.
    pub fn is_available_at_exit(&self, block: u32, expr: &Expression) -> bool {
        self.avail_out.get(&block).map_or(false, |set| set.contains(expr))
    }
    
    /// CAP-AE-06: Find expressions computed when already available (CSE opportunities).
    /// 
    /// Returns: Vec<(expr_text, original_line, redundant_line)>
    pub fn redundant_computations(&self) -> Vec<(String, u32, u32)> {
        // Implementation handles intra-block kills carefully
        // See Python source lines 162-217 for algorithm
    }
    
    /// CAP-AE-10: Get expressions available at a specific source line.
    pub fn get_available_at_line(&self, line: u32, cfg: &CfgInfo) -> HashSet<Expression> {
        for block in &cfg.blocks {
            if block.start_line <= line && line <= block.end_line {
                return self.avail_in.get(&block.id).cloned().unwrap_or_default();
            }
        }
        HashSet::new()
    }
    
    /// CAP-AE-11: Serialize to JSON-compatible structure.
    pub fn to_json(&self) -> serde_json::Value {
        // Output format:
        // {
        //   "avail_in": {"0": [{"text": "a + b", "operands": ["a", "b"], "line": 2}], ...},
        //   "avail_out": {...},
        //   "all_expressions": [...],
        //   "entry_block": 0,
        //   "redundant_computations": [{"expr": "a + b", "first_at": 2, "redundant_at": 4}]
        // }
    }
}
```

### CAP-AE-04, CAP-AE-05: Algorithm (MUST Analysis with Fixpoint)

```rust
/// Compute available expressions using forward MUST dataflow analysis.
/// 
/// Algorithm:
/// 1. Extract all expressions and their operands from DFG
/// 2. Compute gen/kill sets for each block
/// 3. Initialize: entry = {}, others = ALL_EXPRS
/// 4. Iterate until fixed point:
///    - avail_in[b] = intersection(avail_out[p] for p in predecessors[b])
///    - avail_out[b] = gen[b] | (avail_in[b] - killed_by_block[b])
/// 5. Return AvailableExprsInfo with redundant_computations()
/// 
/// FM1 Mitigation: Commutative expression normalization
/// FM2 Mitigation: Function calls excluded from CSE (CAP-AE-12)
/// FM3 Mitigation: ALL_EXPRS computed before init, proper empty set for entry
pub fn compute_available_exprs(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
) -> Result<AvailableExprsInfo, TldrError> {
    // Step 1: Extract expressions from variable uses (pairs on same line)
    let (all_exprs, block_info, expr_instances) = extract_expressions_from_refs(cfg, dfg)?;
    
    // Early return if no expressions
    if all_exprs.is_empty() {
        return Ok(AvailableExprsInfo::empty(cfg.entry_block_id));
    }
    
    // Step 2: Build predecessor map
    let predecessors = build_predecessors(cfg);
    
    // Step 3: Initialize (MUST analysis: start optimistic except entry)
    let mut avail_in: HashMap<u32, HashSet<Expression>> = HashMap::new();
    let mut avail_out: HashMap<u32, HashSet<Expression>> = HashMap::new();
    
    let entry = cfg.entry_block_id;
    avail_in.insert(entry, HashSet::new());  // Nothing available at entry
    avail_out.insert(entry, block_info[&entry].gen.clone());
    
    for block in &cfg.blocks {
        if block.id != entry {
            avail_in.insert(block.id, all_exprs.clone());
            avail_out.insert(block.id, all_exprs.clone());
        }
    }
    
    // Step 4: Iterate until fixed point
    let mut changed = true;
    let max_iterations = cfg.blocks.len() * all_exprs.len() + 10;
    let mut iteration = 0;
    
    while changed && iteration < max_iterations {
        changed = false;
        iteration += 1;
        
        for block in &cfg.blocks {
            if block.id == entry {
                continue;
            }
            
            // avail_in = INTERSECTION of all predecessor's avail_out
            let preds = predecessors.get(&block.id).unwrap_or(&vec![]);
            let new_in = if preds.is_empty() {
                HashSet::new()
            } else {
                preds.iter()
                    .map(|p| avail_out.get(p).unwrap())
                    .fold(avail_out[&preds[0]].clone(), |acc, set| {
                        acc.intersection(set).cloned().collect()  // MUST = intersection
                    })
            };
            
            // avail_out = gen | (avail_in - killed)
            let info = &block_info[&block.id];
            let killed: HashSet<_> = new_in.iter()
                .filter(|e| info.kill.iter().any(|k| e.is_killed_by(k)))
                .cloned()
                .collect();
            let new_out: HashSet<_> = info.gen.union(&new_in.difference(&killed).cloned().collect())
                .cloned()
                .collect();
            
            if avail_in[&block.id] != new_in || avail_out[&block.id] != new_out {
                changed = true;
                avail_in.insert(block.id, new_in);
                avail_out.insert(block.id, new_out);
            }
        }
    }
    
    Ok(AvailableExprsInfo {
        avail_in,
        avail_out,
        all_exprs,
        entry_block: entry,
        expr_instances,
    })
}
```

### Behavioral Contracts

| ID | Contract | Test Case |
|----|----------|-----------|
| CAP-AE-01 | Expression equality based on text only | `expr1(line=1) == expr2(line=5)` if same text |
| CAP-AE-02 | Commutative normalization | `"a + b"` equals `"b + a"` after normalization |
| CAP-AE-03 | Gen: expressions before operand killed | `x=a+b; a=5` -> gen contains `a+b` with original line |
| CAP-AE-04 | MUST semantics: intersection at joins | Diamond CFG: single-branch expr NOT available at merge |
| CAP-AE-05 | Fixpoint terminates | Max iterations = blocks * expressions + 10 |
| CAP-AE-06 | redundant_computations() accuracy | `(expr_text, first_line, redundant_line)` tuples |
| CAP-AE-07 | Intra-block kill handling | `x=a+b; a=5; y=a+b` -> second NOT redundant |
| CAP-AE-08 | is_available() at entry | True iff expr in avail_in[block] |
| CAP-AE-09 | is_available_at_exit() | True iff expr in avail_out[block] |
| CAP-AE-10 | get_available_at_line() | Maps line to containing block's avail_in |
| CAP-AE-11 | to_json() serializable | All fields JSON-compatible |
| CAP-AE-12 | Function calls excluded | `foo(x)` not tracked as available expression |

---

## Part 2: Abstract Interpretation Analysis

### Data Types

#### CAP-AI-01: Nullability Enum

```rust
/// Nullability lattice: NEVER < MAYBE < ALWAYS
/// 
/// Used to track whether a variable may be null/None at a program point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Nullability {
    /// Definitely not null
    Never,
    /// Could be null or non-null
    Maybe,
    /// Definitely null
    Always,
}

impl Default for Nullability {
    fn default() -> Self {
        Nullability::Maybe
    }
}
```

#### CAP-AI-02 to CAP-AI-06: AbstractValue

```rust
/// Abstract representation of a variable's value at a program point.
/// 
/// Tracks four dimensions:
/// - type_: Inferred type (str, int, list, etc.) or None if unknown
/// - range_: Value range [min, max] for numeric types, None for unbounded
/// - nullable: Whether the value can be null/None
/// - constant: If value is a known constant, the value itself
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AbstractValue {
    /// Inferred type name (e.g., "int", "str") or None if unknown
    #[serde(rename = "type")]
    pub type_: Option<String>,
    
    /// Value range [min, max] for numerics. None bounds mean infinity.
    /// For strings, tracks length.
    pub range_: Option<(Option<i64>, Option<i64>)>,
    
    /// Nullability status
    pub nullable: Nullability,
    
    /// Known constant value (not hashed, used for propagation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constant: Option<ConstantValue>,
}

/// Constant values that can be tracked
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConstantValue {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
}

impl AbstractValue {
    /// CAP-AI-04: Top of lattice - no information known
    pub fn top() -> Self {
        AbstractValue {
            type_: None,
            range_: None,
            nullable: Nullability::Maybe,
            constant: None,
        }
    }
    
    /// CAP-AI-04: Bottom of lattice - contradiction (unreachable)
    pub fn bottom() -> Self {
        AbstractValue {
            type_: Some("<bottom>".to_string()),
            range_: Some((None, None)),
            nullable: Nullability::Never,
            constant: None,
        }
    }
    
    /// CAP-AI-03: Create from known constant
    pub fn from_constant(value: ConstantValue) -> Self {
        match value {
            ConstantValue::Int(v) => AbstractValue {
                type_: Some("int".to_string()),
                range_: Some((Some(v), Some(v))),
                nullable: Nullability::Never,
                constant: Some(ConstantValue::Int(v)),
            },
            ConstantValue::Float(v) => AbstractValue {
                type_: Some("float".to_string()),
                range_: None,  // Float ranges less useful
                nullable: Nullability::Never,
                constant: Some(ConstantValue::Float(v)),
            },
            ConstantValue::String(ref s) => AbstractValue {
                type_: Some("str".to_string()),
                range_: Some((Some(s.len() as i64), Some(s.len() as i64))),  // CAP-AI-18: Track length
                nullable: Nullability::Never,
                constant: Some(value),
            },
            ConstantValue::Bool(v) => AbstractValue {
                type_: Some("bool".to_string()),
                range_: Some((Some(v as i64), Some(v as i64))),
                nullable: Nullability::Never,
                constant: Some(ConstantValue::Bool(v)),
            },
            ConstantValue::Null => AbstractValue {
                type_: Some("NoneType".to_string()),
                range_: None,
                nullable: Nullability::Always,
                constant: None,
            },
        }
    }
    
    /// CAP-AI-05: Check if value could be zero (for division check)
    pub fn may_be_zero(&self) -> bool {
        match &self.range_ {
            None => true,  // Unknown range, conservatively true
            Some((low, high)) => {
                let low = low.unwrap_or(i64::MIN);
                let high = high.unwrap_or(i64::MAX);
                low <= 0 && 0 <= high
            }
        }
    }
    
    /// CAP-AI-06: Check if value could be null/None
    pub fn may_be_null(&self) -> bool {
        self.nullable != Nullability::Never
    }
    
    /// Check if this is a known constant value
    pub fn is_constant(&self) -> bool {
        self.constant.is_some()
    }
}
```

#### CAP-AI-07: AbstractState

```rust
/// Abstract state at a program point: mapping from variables to abstract values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbstractState {
    pub values: HashMap<String, AbstractValue>,
}

impl AbstractState {
    /// Get abstract value for variable, defaulting to top (unknown)
    pub fn get(&self, var: &str) -> AbstractValue {
        self.values.get(var).cloned().unwrap_or_else(AbstractValue::top)
    }
    
    /// Return new state with updated variable value (immutable style)
    pub fn set(&self, var: &str, value: AbstractValue) -> Self {
        let mut new_values = self.values.clone();
        new_values.insert(var.to_string(), value);
        AbstractState { values: new_values }
    }
    
    /// Create a copy of this state
    pub fn copy(&self) -> Self {
        self.clone()
    }
}
```

#### CAP-AI-21, CAP-AI-22: AbstractInterpInfo

```rust
/// Abstract interpretation analysis results for a function.
#[derive(Debug, Serialize, Deserialize)]
pub struct AbstractInterpInfo {
    /// Abstract state at entry of each block
    pub state_in: HashMap<u32, AbstractState>,
    
    /// Abstract state at exit of each block
    pub state_out: HashMap<u32, AbstractState>,
    
    /// CAP-AI-10: Potential division-by-zero warnings (line, var)
    pub potential_div_zero: Vec<(u32, String)>,
    
    /// CAP-AI-11: Potential null dereference warnings (line, var)
    pub potential_null_deref: Vec<(u32, String)>,
    
    /// Function name
    pub function_name: String,
}

impl AbstractInterpInfo {
    /// Get abstract value of variable at entry to block
    pub fn value_at(&self, block: u32, var: &str) -> AbstractValue {
        self.state_in.get(&block)
            .map(|s| s.get(var))
            .unwrap_or_else(AbstractValue::top)
    }
    
    /// Get abstract value of variable at exit of block
    pub fn value_at_exit(&self, block: u32, var: &str) -> AbstractValue {
        self.state_out.get(&block)
            .map(|s| s.get(var))
            .unwrap_or_else(AbstractValue::top)
    }
    
    /// Get the value range for variable at block entry
    pub fn range_at(&self, block: u32, var: &str) -> Option<(Option<i64>, Option<i64>)> {
        self.value_at(block, var).range_
    }
    
    /// Get the inferred type for variable at block entry
    pub fn type_at(&self, block: u32, var: &str) -> Option<String> {
        self.value_at(block, var).type_
    }
    
    /// Check if variable is definitely non-null at block entry
    pub fn is_definitely_not_null(&self, block: u32, var: &str) -> bool {
        self.value_at(block, var).nullable == Nullability::Never
    }
    
    /// CAP-AI-12: Get all variables with known constant values at function exit
    pub fn get_constants(&self) -> HashMap<String, ConstantValue> {
        let mut constants = HashMap::new();
        for state in self.state_out.values() {
            for (var, val) in &state.values {
                if let Some(c) = &val.constant {
                    constants.insert(var.clone(), c.clone());
                }
            }
        }
        constants
    }
    
    /// CAP-AI-22: Serialize to JSON-compatible structure
    pub fn to_json(&self) -> serde_json::Value {
        // Output format per v1 CLI
    }
}
```

### CAP-AI-08, CAP-AI-09: State Join and Widening

```rust
/// CAP-AI-08: Join multiple abstract states at a merge point.
/// 
/// For each variable:
/// - range: take union (widest bounds)
/// - type: take common type or unknown
/// - nullable: MAYBE if any is MAYBE, else max
/// - constant: keep only if all agree
fn join_states(states: &[AbstractState]) -> AbstractState {
    if states.is_empty() {
        return AbstractState::default();
    }
    if states.len() == 1 {
        return states[0].clone();
    }
    
    let all_vars: HashSet<_> = states.iter()
        .flat_map(|s| s.values.keys().cloned())
        .collect();
    
    let mut result = HashMap::new();
    for var in all_vars {
        let values: Vec<_> = states.iter().map(|s| s.get(&var)).collect();
        result.insert(var, join_values(&values));
    }
    
    AbstractState { values: result }
}

fn join_values(values: &[AbstractValue]) -> AbstractValue {
    // Range: union (widest bounds)
    let ranges: Vec<_> = values.iter()
        .filter_map(|v| v.range_)
        .collect();
    
    let joined_range = if ranges.is_empty() {
        None
    } else {
        let lows: Vec<_> = ranges.iter().filter_map(|r| r.0).collect();
        let highs: Vec<_> = ranges.iter().filter_map(|r| r.1).collect();
        Some((
            lows.iter().min().copied(),
            highs.iter().max().copied(),
        ))
    };
    
    // Type: common type or None
    let types: Vec<_> = values.iter().filter_map(|v| v.type_.clone()).collect();
    let joined_type = if types.windows(2).all(|w| w[0] == w[1]) && !types.is_empty() {
        Some(types[0].clone())
    } else {
        None
    };
    
    // Nullable: MAYBE if any is MAYBE or disagreement
    let nulls: Vec<_> = values.iter().map(|v| v.nullable).collect();
    let joined_null = if nulls.contains(&Nullability::Maybe) {
        Nullability::Maybe
    } else if nulls.windows(2).all(|w| w[0] == w[1]) {
        nulls[0]
    } else {
        Nullability::Maybe
    };
    
    // Constant: only if all agree
    let constants: Vec<_> = values.iter().filter_map(|v| v.constant.clone()).collect();
    let joined_const = if constants.len() == values.len() 
        && constants.windows(2).all(|w| w[0] == w[1]) {
        constants.into_iter().next()
    } else {
        None
    };
    
    AbstractValue {
        type_: joined_type,
        range_: joined_range,
        nullable: joined_null,
        constant: joined_const,
    }
}

/// CAP-AI-09: Apply widening to ensure termination on loops.
/// 
/// For each variable, if the range is growing, widen to infinity.
fn widen_state(old: &AbstractState, new: &AbstractState) -> AbstractState {
    let all_vars: HashSet<_> = old.values.keys()
        .chain(new.values.keys())
        .cloned()
        .collect();
    
    let mut result = HashMap::new();
    for var in all_vars {
        let old_val = old.get(&var);
        let new_val = new.get(&var);
        result.insert(var, widen_value(&old_val, &new_val));
    }
    
    AbstractState { values: result }
}

fn widen_value(old: &AbstractValue, new: &AbstractValue) -> AbstractValue {
    let widened_range = match (&old.range_, &new.range_) {
        (None, None) => None,
        (None, r) => r.clone(),
        (_, None) => None,
        (Some((old_low, old_high)), Some((new_low, new_high))) => {
            // Widen low: if growing downward, widen to -inf
            let widened_low = match (old_low, new_low) {
                (None, _) => None,  // Already widened
                (_, None) => None,  // Widen
                (Some(o), Some(n)) if *n < *o => None,  // Growing down
                (_, n) => *n,
            };
            
            // Widen high: if growing upward, widen to +inf
            let widened_high = match (old_high, new_high) {
                (None, _) => None,  // Already widened
                (_, None) => None,  // Widen
                (Some(o), Some(n)) if *n > *o => None,  // Growing up
                (_, n) => *n,
            };
            
            Some((widened_low, widened_high))
        }
    };
    
    AbstractValue {
        type_: new.type_.clone(),
        range_: widened_range,
        nullable: new.nullable,
        constant: None,  // Constant lost after widening
    }
}
```

### CAP-AI-13, CAP-AI-14, CAP-AI-19: Transfer Function and RHS Parsing

```rust
/// CAP-AI-14: Parse RHS of assignment and compute abstract value.
/// 
/// Handles:
/// - Integer literals: x = 5
/// - String literals: x = "hello"
/// - None/null: x = None (language-specific keywords)
/// - Boolean: True/False or true/false (language-specific)
/// - Simple arithmetic: x = a + 1, x = b * 2 (CAP-AI-13)
/// - Variable copies: x = y (CAP-AI-19)
pub fn parse_rhs_abstract(
    line: &str,
    var: &str,
    state: &AbstractState,
    language: Language,
) -> AbstractValue {
    // Get language-specific patterns
    let comment_pattern = get_comment_pattern(language);
    let null_keywords = get_null_keywords(language);  // CAP-AI-15
    let bool_keywords = get_boolean_keywords(language);  // CAP-AI-16
    
    // Strip comments (CAP-AI-17)
    let line = comment_pattern.split(line).next().unwrap_or("");
    
    // Find assignment: var = ... or var := ...
    let rhs = extract_rhs(line, var)?;
    
    // Integer literal
    if let Ok(v) = rhs.parse::<i64>() {
        return AbstractValue::from_constant(ConstantValue::Int(v));
    }
    
    // Float literal
    if let Ok(v) = rhs.parse::<f64>() {
        return AbstractValue::from_constant(ConstantValue::Float(v));
    }
    
    // String literal
    if (rhs.starts_with('"') && rhs.ends_with('"')) ||
       (rhs.starts_with('\'') && rhs.ends_with('\'')) {
        let s = rhs[1..rhs.len()-1].to_string();
        return AbstractValue::from_constant(ConstantValue::String(s));
    }
    
    // Null keywords (CAP-AI-15)
    if null_keywords.contains(&rhs) {
        return AbstractValue::from_constant(ConstantValue::Null);
    }
    
    // Boolean keywords (CAP-AI-16)
    if let Some(&b) = bool_keywords.get(rhs) {
        return AbstractValue::from_constant(ConstantValue::Bool(b));
    }
    
    // Variable copy (CAP-AI-19)
    if is_identifier(rhs) {
        return state.get(rhs);
    }
    
    // Simple arithmetic: x = a + N or x = a - N (CAP-AI-13)
    if let Some((operand, op, const_val)) = parse_simple_arithmetic(rhs) {
        return apply_arithmetic(state.get(&operand), op, const_val);
    }
    
    // Unknown RHS
    AbstractValue::top()
}

/// CAP-AI-13: Apply abstract arithmetic
fn apply_arithmetic(operand: AbstractValue, op: char, constant: i64) -> AbstractValue {
    let new_range = operand.range_.map(|(low, high)| {
        match op {
            '+' => (low.map(|l| l + constant), high.map(|h| h + constant)),
            '-' => (low.map(|l| l - constant), high.map(|h| h - constant)),
            '*' => {
                let vals = [
                    low.map(|l| l * constant),
                    high.map(|h| h * constant),
                ];
                (vals.iter().filter_map(|&v| v).min(), 
                 vals.iter().filter_map(|&v| v).max())
            }
            _ => (None, None),  // Unknown op -> unbounded
        }
    });
    
    AbstractValue {
        type_: operand.type_,
        range_: new_range,
        nullable: operand.nullable,
        constant: if operand.is_constant() && new_range.map(|(l, h)| l == h).unwrap_or(false) {
            new_range.and_then(|(l, _)| l.map(ConstantValue::Int))
        } else {
            None
        },
    }
}
```

### CAP-AI-15, CAP-AI-16, CAP-AI-17: Multi-Language Support

```rust
/// CAP-AI-15: Get null keywords for language
fn get_null_keywords(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["None"],
        Language::TypeScript | Language::JavaScript => &["null", "undefined"],
        Language::Go => &["nil"],
        Language::Rust => &[],  // Rust has no null (None is Option::None)
        Language::Java | Language::Kotlin | Language::CSharp => &["null"],
        Language::Swift => &["nil"],
        _ => &["null", "nil", "None"],  // Fallback
    }
}

/// CAP-AI-16: Get boolean keywords for language
fn get_boolean_keywords(language: Language) -> HashMap<&'static str, bool> {
    match language {
        Language::Python => [("True", true), ("False", false)].into(),
        Language::TypeScript | Language::JavaScript | Language::Go | Language::Rust => 
            [("true", true), ("false", false)].into(),
        _ => [("True", true), ("False", false), ("true", true), ("false", false)].into(),
    }
}

/// CAP-AI-17: Get comment pattern for language
fn get_comment_pattern(language: Language) -> &'static str {
    match language {
        Language::Python => "#",
        Language::TypeScript | Language::JavaScript | Language::Go | 
        Language::Rust | Language::Java | Language::CSharp | Language::Kotlin | Language::Swift => "//",
        _ => "#",  // Fallback
    }
}
```

### Main Algorithm

```rust
/// Compute abstract interpretation using forward dataflow with widening.
/// 
/// Algorithm:
/// 1. Initialize entry block with parameter values (top)
/// 2. Iterate in reverse postorder until fixpoint:
///    - state_in[b] = join(state_out[p] for p in predecessors[b])
///    - Apply widening at loop headers (CAP-AI-09)
///    - state_out[b] = transfer(state_in[b], block[b])
/// 3. Detect potential issues:
///    - Division by zero: divisor.may_be_zero() (CAP-AI-10)
///    - Null dereference: obj.may_be_null() at attribute access (CAP-AI-11)
/// 4. Return AbstractInterpInfo
pub fn compute_abstract_interp(
    cfg: &CfgInfo,
    refs: &[VarRef],
    source_lines: Option<&[String]>,
    language: Language,
) -> Result<AbstractInterpInfo, TldrError> {
    let predecessors = build_predecessors(cfg);
    let loop_headers = find_back_edges(cfg);
    let block_order = reverse_postorder(cfg);
    
    // Initialize
    let mut state_in: HashMap<u32, AbstractState> = HashMap::new();
    let mut state_out: HashMap<u32, AbstractState> = HashMap::new();
    
    let entry = cfg.entry_block_id;
    let init_state = init_params(cfg, refs);
    state_in.insert(entry, init_state.clone());
    state_out.insert(entry, init_state);
    
    for block in &cfg.blocks {
        if block.id != entry {
            state_in.insert(block.id, AbstractState::default());
            state_out.insert(block.id, AbstractState::default());
        }
    }
    
    // Iterate until fixpoint
    let mut changed = true;
    let max_iterations = cfg.blocks.len() * 10 + 100;
    let mut iteration = 0;
    
    while changed && iteration < max_iterations {
        changed = false;
        iteration += 1;
        
        for &block_id in &block_order {
            if block_id == entry {
                continue;
            }
            
            let block = cfg.blocks.iter().find(|b| b.id == block_id).unwrap();
            let preds = predecessors.get(&block_id).unwrap_or(&vec![]);
            
            // Join predecessors
            let pred_states: Vec<_> = preds.iter()
                .map(|p| state_out[p].clone())
                .collect();
            let mut new_in = join_states(&pred_states);
            
            // Apply widening at loop headers
            if loop_headers.contains(&block_id) {
                new_in = widen_state(&state_in[&block_id], &new_in);
            }
            
            // Transfer function
            let new_out = transfer_block(&new_in, block, refs, source_lines, language);
            
            if state_in[&block_id] != new_in || state_out[&block_id] != new_out {
                changed = true;
                state_in.insert(block_id, new_in);
                state_out.insert(block_id, new_out);
            }
        }
    }
    
    // Detect issues
    let potential_div_zero = find_div_zero(cfg, refs, &state_in, source_lines, &state_out);  // CAP-AI-20
    let potential_null_deref = find_null_deref(cfg, refs, &state_in);
    
    Ok(AbstractInterpInfo {
        state_in,
        state_out,
        potential_div_zero,
        potential_null_deref,
        function_name: cfg.function_name.clone(),
    })
}
```

### Behavioral Contracts

| ID | Contract | Test Case |
|----|----------|-----------|
| CAP-AI-01 | Nullability enum values | NEVER="never", MAYBE="maybe", ALWAYS="always" |
| CAP-AI-02 | AbstractValue is frozen/hashable | Can be used in HashSet |
| CAP-AI-03 | from_constant(5) -> range=[5,5], constant=5 | Integer constant |
| CAP-AI-04 | top() -> None type/range, MAYBE nullable | Unknown value |
| CAP-AI-05 | may_be_zero(): range [-5,5] -> true | Division check |
| CAP-AI-06 | may_be_null(): NEVER -> false | Null check |
| CAP-AI-07 | AbstractState.get() -> top() for missing | Default behavior |
| CAP-AI-08 | Join [1,1] and [10,10] -> [1,10] | Range union |
| CAP-AI-09 | Growing upper bound -> widen to +inf | Loop termination |
| CAP-AI-10 | Division by may_be_zero() divisor flagged | Safety check |
| CAP-AI-11 | Attribute access on may_be_null() flagged | Safety check |
| CAP-AI-12 | Constant propagation through arithmetic | x=5; y=x+1 -> y=[6,6] |
| CAP-AI-13 | Arithmetic on ranges | [5,5] + 3 -> [8,8] |
| CAP-AI-14 | RHS parsing: int, float, str, None | Literal extraction |
| CAP-AI-15 | Python None, TS null/undefined, Go nil | Multi-language null |
| CAP-AI-16 | Python True/False, TS true/false | Multi-language bool |
| CAP-AI-17 | Python #, TS // comment stripping | Parse accuracy |
| CAP-AI-18 | String "hello" -> range=[5,5] (length) | String tracking |
| CAP-AI-19 | y = x copies abstract value | Variable flow |
| CAP-AI-20 | x=5; y=1/x no warning (intra-block) | False positive prevention |
| CAP-AI-21 | Query methods return correct values | API contract |
| CAP-AI-22 | to_json() JSON-serializable | CLI output |

---

## CLI Interface

### Available Expressions Command

```bash
tldr available <file> <function> [--lang <lang>] [--check <expr>] [--at-line <n>]
```

**Output format:**
```json
{
  "function": "example",
  "avail_in": {
    "0": [],
    "1": [{"text": "a + b", "operands": ["a", "b"], "line": 2}]
  },
  "avail_out": {
    "0": [{"text": "a + b", "operands": ["a", "b"], "line": 2}],
    "1": [{"text": "a + b", "operands": ["a", "b"], "line": 2}]
  },
  "all_expressions": [
    {"text": "a + b", "operands": ["a", "b"], "line": 2}
  ],
  "redundant_computations": [
    {"expr": "a + b", "first_at": 2, "redundant_at": 4}
  ]
}
```

### Abstract Interpretation Command

```bash
tldr abstract-interp <file> <function> [--lang <lang>] [--var <name>] [--line <n>]
```

**Output format:**
```json
{
  "function": "example",
  "state_in": {
    "0": {},
    "1": {
      "x": {"type": "int", "range": [5, 5], "nullable": "never", "constant": 5}
    }
  },
  "state_out": {
    "0": {
      "x": {"type": "int", "range": [5, 5], "nullable": "never", "constant": 5}
    }
  },
  "potential_div_zero": [
    {"line": 10, "var": "divisor"}
  ],
  "potential_null_deref": [
    {"line": 15, "var": "obj"}
  ]
}
```

---

## Error Handling

```rust
/// Dataflow analysis errors
#[derive(Debug, thiserror::Error)]
pub enum DataflowError {
    #[error("CFG has no blocks")]
    EmptyCfg,
    
    #[error("Entry block {0} not found in CFG")]
    EntryBlockNotFound(u32),
    
    #[error("Analysis did not converge after {0} iterations")]
    NoConvergence(usize),
    
    #[error("Source line {0} out of range")]
    LineOutOfRange(u32),
    
    #[error("Block {0} not found")]
    BlockNotFound(u32),
}
```

---

## Edge Cases (from tests)

### Available Expressions

1. **Empty function** - No expressions, empty avail_in/avail_out
2. **Unreachable block** - Still included in results (entry/exit present)
3. **Self-loop** - Handles gracefully without infinite loop
4. **Multiple expressions** - Tracked independently
5. **Diamond CFG single branch** - Expression NOT available at merge (MUST)
6. **Diamond CFG both branches** - Expression IS available at merge

### Abstract Interpretation

1. **Empty function** - Empty state, no warnings
2. **Unknown RHS** - Defaults to top()
3. **Parameter starts as top** - Unknown input
4. **Negative constant** - Range [-5, -5]
5. **String constant tracks length** - "hello" -> [5, 5]
6. **Nested loops** - Terminates via widening
7. **Rust has no null keyword** - None is Option::None, not null

---

## Implementation Phases

### Phase 1: Foundation (types.rs)
- [ ] BlockId type alias
- [ ] build_predecessors() helper
- [ ] find_back_edges() helper
- [ ] reverse_postorder() helper

### Phase 2: Available Expressions (available.rs)
- [ ] Expression struct (CAP-AE-01)
- [ ] normalize_expression() (CAP-AE-02)
- [ ] BlockExpressions (CAP-AE-03)
- [ ] AvailableExprsInfo (CAP-AE-08-11)
- [ ] compute_available_exprs() (CAP-AE-04-05)
- [ ] redundant_computations() (CAP-AE-06-07)
- [ ] Function call exclusion (CAP-AE-12)

### Phase 3: Abstract Interpretation (abstract_interp.rs)
- [ ] Nullability enum (CAP-AI-01)
- [ ] AbstractValue struct (CAP-AI-02-06)
- [ ] AbstractState struct (CAP-AI-07)
- [ ] join_states(), widen_state() (CAP-AI-08-09)
- [ ] parse_rhs_abstract() (CAP-AI-14)
- [ ] apply_arithmetic() (CAP-AI-13)
- [ ] Multi-language keywords (CAP-AI-15-17)
- [ ] compute_abstract_interp()
- [ ] find_div_zero(), find_null_deref() (CAP-AI-10-11)
- [ ] AbstractInterpInfo (CAP-AI-21-22)

### Phase 4: Integration
- [ ] Add `pub mod dataflow;` to lib.rs
- [ ] Re-export main types
- [ ] CLI commands in tldr-cli

### Phase 5: Testing
- [ ] Port all 32 available_exprs tests
- [ ] Port all 56 abstract_interp tests
- [ ] Coverage target: 80%

---

## Success Criteria

1. All 34 capabilities implemented
2. All 88 test cases ported and passing
3. JSON output matches v1 CLI format
4. Multi-language support (Python, TypeScript, Go, Rust)
5. Performance: O(blocks * expressions) for available, O(blocks * vars * iterations) for abstract
6. No panics on edge cases (empty CFG, unreachable blocks, self-loops)

---

## References

- Dragon Book Ch. 10: Available Expressions
- Cooper & Torczon: "Engineering a Compiler" Ch. 9
- Cousot & Cousot (1977): "Abstract Interpretation: A Unified Lattice Model"
- Python source: packages/tldr-code/available_exprs.py (538 lines)
- Python source: packages/tldr-code/abstract_interp.py (980 lines)
- Gap analysis: thoughts/shared/gap-analysis/session13-available-abstract-gap.yaml
