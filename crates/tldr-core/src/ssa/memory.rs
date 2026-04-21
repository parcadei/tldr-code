//! Memory SSA
//!
//! Extends SSA to track memory state changes for heap operations.
//! Uses LLVM-style single memory variable (not partitioned by object).
//!
//! # Design
//!
//! Memory SSA treats memory as a single versioned variable. Each store
//! creates a new memory version, and each load uses the current memory
//! version. At control flow merge points, MemoryPhi nodes select the
//! appropriate memory version from predecessors.
//!
//! This is a conservative approach - all stores are assumed to potentially
//! alias all loads. More precise analysis would require points-to analysis.
//!
//! # References
//!
//! - LLVM MemorySSA documentation
//! - "Memory SSA - A Unified Approach for Sparsely Representing Memory Operations"

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::types::CfgInfo;
use crate::TldrResult;

use super::dominators::{build_dominator_tree, compute_dominance_frontier, DominanceFrontier};
use super::types::{SsaFunction, SsaInstructionKind};

// =============================================================================
// Memory SSA Types
// =============================================================================

/// Memory version identifier
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryVersion(pub u32);

impl std::fmt::Display for MemoryVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mem_{}", self.0)
    }
}

/// Memory SSA for heap analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySsa {
    /// Function name
    pub function: String,
    /// File path (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Memory phi nodes at merge points
    pub memory_phis: Vec<MemoryPhi>,
    /// Memory definitions (stores)
    pub memory_defs: Vec<MemoryDef>,
    /// Memory uses (loads)
    pub memory_uses: Vec<MemoryUse>,
    /// Memory def-use chains: for each def version, list of use versions
    pub def_use: HashMap<MemoryVersion, Vec<MemoryVersion>>,
    /// Statistics
    pub stats: MemorySsaStats,
}

/// Memory SSA statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemorySsaStats {
    /// Number of memory definitions (stores)
    pub defs: usize,
    /// Number of memory uses (loads)
    pub uses: usize,
    /// Number of memory phi functions
    pub phis: usize,
    /// Next available memory version
    pub max_version: u32,
}

/// Memory phi node at a merge point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPhi {
    /// Result memory version
    pub result: MemoryVersion,
    /// Block where this phi is placed
    pub block: usize,
    /// Sources from predecessors
    pub sources: Vec<MemoryPhiSource>,
}

/// Source for a memory phi
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPhiSource {
    /// Predecessor block ID
    pub block: usize,
    /// Memory version from that predecessor
    pub version: MemoryVersion,
}

/// Memory definition (store operation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDef {
    /// Memory version created by this store
    pub version: MemoryVersion,
    /// Previous memory version (clobbered)
    pub clobbers: MemoryVersion,
    /// Block containing this store
    pub block: usize,
    /// Line number
    pub line: u32,
    /// Access description (e.g., "x.field" or "arr[i]")
    pub access: String,
    /// Kind of memory operation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<MemoryDefKind>,
}

/// Kind of memory definition
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryDefKind {
    /// Direct store: obj.field = value
    Store,
    /// Function call that may modify memory
    Call,
    /// Allocation: x = SomeClass()
    Alloc,
}

/// Memory use (load operation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUse {
    /// Memory version used by this load
    pub version: MemoryVersion,
    /// Block containing this load
    pub block: usize,
    /// Line number
    pub line: u32,
    /// Access description
    pub access: String,
    /// Kind of memory operation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<MemoryUseKind>,
}

/// Kind of memory use
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryUseKind {
    /// Direct load: value = obj.field
    Load,
    /// Function call that reads memory
    Call,
}

// =============================================================================
// Memory SSA Construction State
// =============================================================================

/// State for Memory SSA construction
struct MemorySsaBuilder {
    /// Next available memory version
    next_version: u32,
    /// Current memory version at each block (after processing)
    block_out_version: HashMap<usize, MemoryVersion>,
    /// Memory version stack for renaming (like scalar SSA)
    version_stack: Vec<MemoryVersion>,
    /// Collected memory definitions
    memory_defs: Vec<MemoryDef>,
    /// Collected memory uses
    memory_uses: Vec<MemoryUse>,
    /// Memory phi functions
    memory_phis: Vec<MemoryPhi>,
    /// Blocks with memory definitions (for phi placement)
    def_blocks: HashSet<usize>,
}

impl MemorySsaBuilder {
    fn new() -> Self {
        MemorySsaBuilder {
            next_version: 1, // Version 0 is the initial/undefined state
            block_out_version: HashMap::new(),
            version_stack: vec![MemoryVersion(0)], // Initial memory state
            memory_defs: Vec::new(),
            memory_uses: Vec::new(),
            memory_phis: Vec::new(),
            def_blocks: HashSet::new(),
        }
    }

    /// Allocate a new memory version
    fn new_version(&mut self) -> MemoryVersion {
        let version = MemoryVersion(self.next_version);
        self.next_version += 1;
        version
    }

    /// Get current memory version (top of stack)
    fn current_version(&self) -> MemoryVersion {
        *self.version_stack.last().unwrap_or(&MemoryVersion(0))
    }

    /// Push a new version onto the stack
    fn push_version(&mut self, version: MemoryVersion) {
        self.version_stack.push(version);
    }

    /// Pop a version from the stack
    fn pop_version(&mut self) {
        if self.version_stack.len() > 1 {
            self.version_stack.pop();
        }
    }

    /// Record a memory definition (store)
    fn add_def(&mut self, block: usize, line: u32, access: String, kind: MemoryDefKind) {
        let clobbers = self.current_version();
        let version = self.new_version();

        self.memory_defs.push(MemoryDef {
            version,
            clobbers,
            block,
            line,
            access,
            kind: Some(kind),
        });

        self.push_version(version);
        self.def_blocks.insert(block);
    }

    /// Record a memory use (load)
    fn add_use(&mut self, block: usize, line: u32, access: String, kind: MemoryUseKind) {
        let version = self.current_version();

        self.memory_uses.push(MemoryUse {
            version,
            block,
            line,
            access,
            kind: Some(kind),
        });
    }

    /// Add a memory phi function at a block
    fn add_phi(&mut self, block: usize) -> MemoryVersion {
        let version = self.new_version();

        self.memory_phis.push(MemoryPhi {
            result: version,
            block,
            sources: Vec::new(), // Filled in during renaming
        });

        version
    }
}

// =============================================================================
// Memory SSA Construction
// =============================================================================

/// Build Memory SSA for heap operations
///
/// # Arguments
/// * `cfg` - Control flow graph
/// * `ssa` - Pre-constructed SSA form for scalar variables
///
/// # Returns
/// * `MemorySsa` - Memory SSA representation
///
/// # Design (LLVM-style)
/// - Single memory variable versioned at each store
/// - MemoryPhi nodes at merge points
/// - Memory def-use chains connect loads to stores
/// - Conservative: assume all stores may alias all loads
/// - Function calls treated as clobbering memory
///
/// # Algorithm
/// 1. Identify memory operations from SSA instructions
///    - Attribute access (obj.field) = store/load
///    - Function calls = clobber (store)
///    - Subscript access (arr[i]) = store/load
/// 2. Place memory phi functions at IDF of def blocks
/// 3. Rename memory versions using dominator tree traversal
/// 4. Build memory def-use chains
pub fn build_memory_ssa(cfg: &CfgInfo, ssa: &SsaFunction) -> TldrResult<MemorySsa> {
    let mut builder = MemorySsaBuilder::new();

    // Phase 1: Extract memory operations from SSA
    let memory_ops = extract_memory_operations(ssa);

    // If no memory operations, return empty Memory SSA
    if memory_ops.is_empty() {
        return Ok(MemorySsa {
            function: ssa.function.clone(),
            file: Some(ssa.file.to_string_lossy().to_string()),
            memory_phis: Vec::new(),
            memory_defs: Vec::new(),
            memory_uses: Vec::new(),
            def_use: HashMap::new(),
            stats: MemorySsaStats::default(),
        });
    }

    // Phase 2: Build dominator tree and dominance frontier
    let dom_tree = build_dominator_tree(cfg)?;
    let dom_frontier = compute_dominance_frontier(cfg, &dom_tree)?;

    // Phase 3: Find blocks with memory definitions
    let def_blocks: HashSet<usize> = memory_ops
        .iter()
        .filter(|op| op.is_def)
        .map(|op| op.block)
        .collect();

    // Phase 4: Place memory phi functions at IDF
    let phi_blocks = place_memory_phis(&def_blocks, &dom_frontier);

    // Create phi functions for each phi block
    let mut phi_versions: HashMap<usize, MemoryVersion> = HashMap::new();
    for &block in &phi_blocks {
        let version = builder.add_phi(block);
        phi_versions.insert(block, version);
    }

    // Phase 5: Rename memory versions
    // Process blocks in dominator tree order
    rename_memory_versions(
        cfg.entry_block,
        cfg,
        &memory_ops,
        &phi_versions,
        &dom_tree,
        &mut builder,
    );

    // Phase 6: Fill in phi sources
    fill_memory_phi_sources(cfg, &mut builder);

    // Phase 7: Build def-use chains
    let def_use = build_memory_def_use_chains(&builder);

    // Compute stats
    let stats = MemorySsaStats {
        defs: builder.memory_defs.len(),
        uses: builder.memory_uses.len(),
        phis: builder.memory_phis.len(),
        max_version: builder.next_version - 1,
    };

    Ok(MemorySsa {
        function: ssa.function.clone(),
        file: Some(ssa.file.to_string_lossy().to_string()),
        memory_phis: builder.memory_phis,
        memory_defs: builder.memory_defs,
        memory_uses: builder.memory_uses,
        def_use,
        stats,
    })
}

/// Intermediate representation of a memory operation
struct MemoryOp {
    block: usize,
    line: u32,
    access: String,
    is_def: bool,
    kind: MemoryOpKind,
}

enum MemoryOpKind {
    Store,
    Load,
    Call,
    Alloc,
}

/// Extract memory operations from SSA instructions
fn extract_memory_operations(ssa: &SsaFunction) -> Vec<MemoryOp> {
    let mut ops = Vec::new();

    for block in &ssa.blocks {
        for instr in &block.instructions {
            // Check instruction kind for memory operations
            match instr.kind {
                SsaInstructionKind::Call => {
                    // Function calls may both read and write memory
                    // Conservative: treat as clobber (def)
                    let access = instr
                        .source_text
                        .as_ref()
                        .map(|s| extract_call_name(s))
                        .unwrap_or_else(|| "call".to_string());

                    ops.push(MemoryOp {
                        block: block.id,
                        line: instr.line,
                        access,
                        is_def: true,
                        kind: MemoryOpKind::Call,
                    });
                }
                SsaInstructionKind::Assign => {
                    // Check if assignment involves attribute access
                    if let Some(source) = &instr.source_text {
                        if is_attribute_access(source) {
                            // Determine if store or load based on assignment direction
                            let (access, is_store) = parse_attribute_assignment(source);

                            if is_store {
                                ops.push(MemoryOp {
                                    block: block.id,
                                    line: instr.line,
                                    access,
                                    is_def: true,
                                    kind: MemoryOpKind::Store,
                                });
                            } else {
                                ops.push(MemoryOp {
                                    block: block.id,
                                    line: instr.line,
                                    access,
                                    is_def: false,
                                    kind: MemoryOpKind::Load,
                                });
                            }
                        } else if is_allocation(source) {
                            // Object allocation
                            let access = extract_allocation(source);
                            ops.push(MemoryOp {
                                block: block.id,
                                line: instr.line,
                                access,
                                is_def: true,
                                kind: MemoryOpKind::Alloc,
                            });
                        }
                    }
                }
                _ => {
                    // Other instruction kinds don't directly affect memory
                }
            }
        }
    }

    ops
}

/// Check if source text contains attribute access (obj.field)
fn is_attribute_access(source: &str) -> bool {
    // Look for patterns like:
    // - obj.field = value (store)
    // - x = obj.field (load)
    // - obj[index] = value (store)
    // - x = obj[index] (load)
    source.contains('.') || source.contains('[')
}

/// Check if source is an allocation (e.g., `x = ClassName()`)
fn is_allocation(source: &str) -> bool {
    // Look for constructor-like patterns
    // Python: ClassName()
    // TypeScript/JS: new ClassName()
    source.contains("new ")
        || (source.contains('(')
            && source.contains(')')
            && !source.starts_with("def ")
            && !source.starts_with("fn ")
            && source
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false))
}

/// Parse attribute assignment to determine access and direction
fn parse_attribute_assignment(source: &str) -> (String, bool) {
    // Split on '=' to determine direction
    if let Some(eq_pos) = source.find('=') {
        let lhs = source[..eq_pos].trim();
        let rhs = source[eq_pos + 1..].trim();

        // If LHS contains '.' or '[', it's a store
        if lhs.contains('.') || lhs.contains('[') {
            return (lhs.to_string(), true);
        }

        // If RHS contains '.' or '[', it's a load
        if rhs.contains('.') || rhs.contains('[') {
            // Extract the attribute access from RHS
            return (extract_access(rhs), false);
        }
    }

    // Default: treat as load
    (source.to_string(), false)
}

/// Extract the attribute access part from an expression
fn extract_access(expr: &str) -> String {
    // Find the first attribute or subscript access
    let trimmed = expr.trim();

    // Handle simple cases
    if let Some(dot_pos) = trimmed.find('.') {
        // Find the start of the base object
        let start = trimmed[..dot_pos]
            .rfind(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|i| i + 1)
            .unwrap_or(0);

        // Find the end of the field name
        let after_dot = dot_pos + 1;
        let end = trimmed[after_dot..]
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|i| after_dot + i)
            .unwrap_or(trimmed.len());

        return trimmed[start..end].to_string();
    }

    if let Some(bracket_pos) = trimmed.find('[') {
        // Find the start of the base and include the subscript
        let start = trimmed[..bracket_pos]
            .rfind(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|i| i + 1)
            .unwrap_or(0);

        let end = trimmed.find(']').map(|i| i + 1).unwrap_or(trimmed.len());

        return trimmed[start..end].to_string();
    }

    trimmed.to_string()
}

/// Extract call name from source text
fn extract_call_name(source: &str) -> String {
    // Extract function name from call expression
    if let Some(paren_pos) = source.find('(') {
        let before_paren = source[..paren_pos].trim();
        // Handle method calls: obj.method()
        if let Some(dot_pos) = before_paren.rfind('.') {
            return before_paren[dot_pos + 1..].to_string();
        }
        // Handle simple calls: func()
        if let Some(eq_pos) = before_paren.rfind('=') {
            return before_paren[eq_pos + 1..].trim().to_string();
        }
        return before_paren.to_string();
    }
    "call".to_string()
}

/// Extract allocation target
fn extract_allocation(source: &str) -> String {
    // Extract the constructor name
    if let Some(new_pos) = source.find("new ") {
        let after_new = &source[new_pos + 4..];
        if let Some(paren_pos) = after_new.find('(') {
            return format!("new {}", &after_new[..paren_pos].trim());
        }
    }

    // Python-style: ClassName()
    if let Some(paren_pos) = source.find('(') {
        let before_paren = source[..paren_pos].trim();
        if let Some(eq_pos) = before_paren.rfind('=') {
            let class_name = before_paren[eq_pos + 1..].trim();
            return format!("new {}", class_name);
        }
    }

    "alloc".to_string()
}

/// Place memory phi functions at iterated dominance frontier
fn place_memory_phis(
    def_blocks: &HashSet<usize>,
    dom_frontier: &DominanceFrontier,
) -> HashSet<usize> {
    dom_frontier.iterated(def_blocks)
}

/// Rename memory versions during dominator tree traversal
#[allow(clippy::only_used_in_recursion)]
fn rename_memory_versions(
    block_id: usize,
    cfg: &CfgInfo,
    memory_ops: &[MemoryOp],
    phi_versions: &HashMap<usize, MemoryVersion>,
    dom_tree: &super::dominators::DominatorTree,
    builder: &mut MemorySsaBuilder,
) {
    // Remember stack depth for popping later
    let stack_depth = builder.version_stack.len();

    // If this block has a memory phi, push its result version
    if let Some(&phi_version) = phi_versions.get(&block_id) {
        builder.push_version(phi_version);
    }

    // Process memory operations in this block
    let block_ops: Vec<_> = memory_ops
        .iter()
        .filter(|op| op.block == block_id)
        .collect();

    for op in block_ops {
        match op.kind {
            MemoryOpKind::Store => {
                builder.add_def(block_id, op.line, op.access.clone(), MemoryDefKind::Store);
            }
            MemoryOpKind::Load => {
                builder.add_use(block_id, op.line, op.access.clone(), MemoryUseKind::Load);
            }
            MemoryOpKind::Call => {
                // Calls may read memory first, then clobber
                builder.add_use(block_id, op.line, op.access.clone(), MemoryUseKind::Call);
                builder.add_def(block_id, op.line, op.access.clone(), MemoryDefKind::Call);
            }
            MemoryOpKind::Alloc => {
                builder.add_def(block_id, op.line, op.access.clone(), MemoryDefKind::Alloc);
            }
        }
    }

    // Record the outgoing memory version for this block
    builder
        .block_out_version
        .insert(block_id, builder.current_version());

    // Recursively process dominated children
    if let Some(node) = dom_tree.nodes.get(&block_id) {
        for &child in &node.children {
            rename_memory_versions(child, cfg, memory_ops, phi_versions, dom_tree, builder);
        }
    }

    // Pop versions pushed in this block
    while builder.version_stack.len() > stack_depth {
        builder.pop_version();
    }
}

/// Fill in memory phi sources from predecessor blocks
fn fill_memory_phi_sources(cfg: &CfgInfo, builder: &mut MemorySsaBuilder) {
    // Build predecessor map
    let mut predecessors: HashMap<usize, Vec<usize>> = HashMap::new();
    for block in &cfg.blocks {
        predecessors.entry(block.id).or_default();
    }
    for edge in &cfg.edges {
        predecessors.entry(edge.to).or_default().push(edge.from);
    }

    // Fill in phi sources
    for phi in &mut builder.memory_phis {
        if let Some(preds) = predecessors.get(&phi.block) {
            for &pred_block in preds {
                // Get the memory version at the end of the predecessor
                let version = builder
                    .block_out_version
                    .get(&pred_block)
                    .copied()
                    .unwrap_or(MemoryVersion(0));

                phi.sources.push(MemoryPhiSource {
                    block: pred_block,
                    version,
                });
            }
        }
    }
}

/// Build memory def-use chains
fn build_memory_def_use_chains(
    builder: &MemorySsaBuilder,
) -> HashMap<MemoryVersion, Vec<MemoryVersion>> {
    let mut chains: HashMap<MemoryVersion, Vec<MemoryVersion>> = HashMap::new();

    // Initialize chains for each def
    for def in &builder.memory_defs {
        chains.entry(def.version).or_default();
    }

    // Also for phi results
    for phi in &builder.memory_phis {
        chains.entry(phi.result).or_default();
    }

    // Add uses to chains - each use references its reaching def
    for use_ in &builder.memory_uses {
        if let Some(uses) = chains.get_mut(&use_.version) {
            // Record that this def reaches this use
            // We use the use's version as an identifier
            uses.push(use_.version);
        }
    }

    // Add phi operands as "uses" of their source versions
    for phi in &builder.memory_phis {
        for source in &phi.sources {
            if let Some(uses) = chains.get_mut(&source.version) {
                uses.push(phi.result);
            }
        }
    }

    chains
}

// =============================================================================
// Memory SSA Queries
// =============================================================================

/// Get the memory version reaching a given point
///
/// # Arguments
/// * `memory_ssa` - Memory SSA representation
/// * `block` - Block ID
/// * `line` - Line number
///
/// # Returns
/// * The memory version that reaches this point, or None if not found
pub fn get_reaching_memory_version(
    memory_ssa: &MemorySsa,
    block: usize,
    line: u32,
) -> Option<MemoryVersion> {
    // Find the last memory def before this line in this block
    let mut latest_version = None;
    let mut latest_line = 0u32;

    // Check memory defs in this block
    for def in &memory_ssa.memory_defs {
        if def.block == block && def.line < line && def.line >= latest_line {
            latest_version = Some(def.version);
            latest_line = def.line;
        }
    }

    // Check memory phis in this block (they're at the start)
    for phi in &memory_ssa.memory_phis {
        if phi.block == block && latest_version.is_none() {
            latest_version = Some(phi.result);
        }
    }

    // If nothing found in this block, we'd need to look at predecessors
    // (for simplicity, return what we found or None)
    latest_version
}

/// Check if a load may alias with a store
///
/// Conservative: assumes everything may alias in the single-memory model
pub fn may_alias(_store: &MemoryDef, _load: &MemoryUse) -> bool {
    // Conservative: assume everything may alias
    // A more precise analysis would check:
    // - Different base objects
    // - Different array indices (if known)
    // - Type-based alias analysis
    true
}

/// Get all memory definitions that reach a given use
pub fn get_reaching_defs_for_use<'a>(
    memory_ssa: &'a MemorySsa,
    use_: &MemoryUse,
) -> Vec<&'a MemoryDef> {
    // In single-memory model, the use's version directly identifies its reaching def
    memory_ssa
        .memory_defs
        .iter()
        .filter(|def| def.version == use_.version)
        .collect()
}

/// Get all uses that a memory definition reaches
pub fn get_uses_for_def<'a>(memory_ssa: &'a MemorySsa, def: &MemoryDef) -> Vec<&'a MemoryUse> {
    // Find all uses that reference this def's version
    memory_ssa
        .memory_uses
        .iter()
        .filter(|use_| use_.version == def.version)
        .collect()
}

// =============================================================================
// Memory Def-Use Chain Types (SSA-17)
// =============================================================================

/// Memory def-use chain for a single memory definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDefUseChain {
    /// The memory definition
    pub def: MemoryVersion,
    /// Line of the definition
    pub def_line: u32,
    /// Block of the definition
    pub def_block: usize,
    /// All uses reached by this definition
    pub uses: Vec<MemoryUseLocation>,
}

/// Location of a memory use
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUseLocation {
    /// Line number
    pub line: u32,
    /// Block ID
    pub block: usize,
}

/// Build explicit memory def-use chains from Memory SSA
pub fn build_explicit_def_use_chains(memory_ssa: &MemorySsa) -> Vec<MemoryDefUseChain> {
    let mut chains = Vec::new();

    for def in &memory_ssa.memory_defs {
        let uses: Vec<MemoryUseLocation> = memory_ssa
            .memory_uses
            .iter()
            .filter(|u| u.version == def.version)
            .map(|u| MemoryUseLocation {
                line: u.line,
                block: u.block,
            })
            .collect();

        chains.push(MemoryDefUseChain {
            def: def.version,
            def_line: def.line,
            def_block: def.block,
            uses,
        });
    }

    chains
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_version_display() {
        let v = MemoryVersion(42);
        assert_eq!(format!("{}", v), "mem_42");
    }

    #[test]
    fn test_memory_version_default() {
        let v = MemoryVersion::default();
        assert_eq!(v.0, 0);
    }

    #[test]
    fn test_is_attribute_access() {
        assert!(is_attribute_access("obj.field = 1"));
        assert!(is_attribute_access("x = obj.field"));
        assert!(is_attribute_access("arr[0] = 1"));
        assert!(is_attribute_access("x = arr[i]"));
        assert!(!is_attribute_access("x = 1"));
    }

    #[test]
    fn test_parse_attribute_assignment_store() {
        let (access, is_store) = parse_attribute_assignment("obj.field = 1");
        assert!(is_store);
        assert_eq!(access, "obj.field");
    }

    #[test]
    fn test_parse_attribute_assignment_load() {
        let (access, is_store) = parse_attribute_assignment("x = obj.field");
        assert!(!is_store);
        assert!(access.contains("obj.field") || access.contains("obj"));
    }

    #[test]
    fn test_extract_call_name() {
        assert_eq!(extract_call_name("x = foo()"), "foo");
        assert_eq!(extract_call_name("obj.method()"), "method");
        assert_eq!(extract_call_name("result = bar(1, 2)"), "bar");
    }
}
