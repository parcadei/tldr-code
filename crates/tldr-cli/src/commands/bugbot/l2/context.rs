//! L2Context -- shared context for all L2 analysis engines.
//!
//! Provides function-level change data (changed, inserted, deleted functions),
//! file contents for both baseline and current revisions, and project-wide
//! configuration. Includes DashMap-based caches for CFG, DFG, SSA, and
//! contracts data, plus OnceLock-backed call graph and change impact fields.
//!
//! # Daemon Integration (Phase 8.4)
//!
//! L2Context carries an optional daemon client that routes IR queries through
//! the daemon's QueryCache when available, falling back to on-the-fly
//! construction when no daemon is running. The daemon field is populated via
//! the `with_daemon` builder method.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use dashmap::DashMap;

use tldr_core::ssa::SsaFunction;
use tldr_core::{CfgInfo, ChangeImpactReport, DfgInfo, Language, ProjectCallGraph};

use super::daemon_client::{DaemonClient, NoDaemon};
use super::types::FunctionId;
use crate::commands::contracts::types::ContractsReport;
use crate::commands::remaining::types::ASTChange;

/// A function that changed between baseline and current revisions.
#[derive(Debug, Clone)]
pub struct FunctionChange {
    /// Unique identifier for this function.
    pub id: FunctionId,
    /// Human-readable function name.
    pub name: String,
    /// Source code in the baseline revision.
    pub old_source: String,
    /// Source code in the current revision.
    pub new_source: String,
}

/// A function that was inserted (no baseline equivalent).
#[derive(Debug, Clone)]
pub struct InsertedFunction {
    /// Unique identifier for this function.
    pub id: FunctionId,
    /// Human-readable function name.
    pub name: String,
    /// Source code of the inserted function.
    pub source: String,
}

/// A function present in baseline but absent in current revision.
#[derive(Debug, Clone)]
pub struct DeletedFunction {
    /// Unique identifier for this function.
    pub id: FunctionId,
    /// Human-readable function name.
    pub name: String,
}

/// Version discriminator for contracts cache.
///
/// Used to distinguish between baseline and current versions when caching
/// analysis results (e.g., pre-/post-conditions).
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum ContractVersion {
    /// The baseline (pre-change) revision.
    Baseline,
    /// The current (post-change) revision.
    Current,
}

/// The function-level diff between baseline and current revisions.
///
/// Groups the three categories of function changes: modified, inserted, and
/// deleted. Extracted as a separate struct to keep `L2Context::new` under
/// clippy's argument limit while maintaining a flat public API on the context.
#[derive(Debug, Clone)]
pub struct FunctionDiff {
    /// Functions whose bodies changed between revisions.
    pub changed: Vec<FunctionChange>,
    /// Functions present in current but not in baseline.
    pub inserted: Vec<InsertedFunction>,
    /// Functions present in baseline but not in current.
    pub deleted: Vec<DeletedFunction>,
}

/// Shared context for all L2 analysis engines.
///
/// Carries the project root, detected language, lists of changed/inserted/deleted
/// functions, and the full file contents for both revisions. Includes lazy-initialized
/// DashMap caches for per-function CFG, DFG, SSA, and contracts data, plus
/// OnceLock-backed project-level call graph and change impact report.
///
/// The daemon client routes IR queries through the daemon's QueryCache when
/// available, falling back to on-the-fly construction via the local caches.
pub struct L2Context {
    /// Absolute path to the project root.
    pub project: PathBuf,
    /// Detected (or user-specified) programming language.
    pub language: Language,
    /// Files that have changes between baseline and current.
    pub changed_files: Vec<PathBuf>,
    /// Function-level diff between baseline and current revisions.
    pub function_diff: FunctionDiff,
    /// Full file contents for the baseline revision, keyed by path.
    pub baseline_contents: HashMap<PathBuf, String>,
    /// Full file contents for the current revision, keyed by path.
    pub current_contents: HashMap<PathBuf, String>,
    /// AST-level changes per file from the diff phase.
    ///
    /// Maps each changed file to its list of `ASTChange` entries (Insert, Update,
    /// Delete). Used by DeltaEngine for finding extractors that need node-level
    /// diff data (e.g., `param-renamed`, `signature-regression`).
    pub ast_changes: HashMap<PathBuf, Vec<ASTChange>>,
    /// Per-function CFG cache (Sync-safe via DashMap).
    cfg_cache: DashMap<FunctionId, CfgInfo>,
    /// Per-function DFG cache (Sync-safe via DashMap).
    dfg_cache: DashMap<FunctionId, DfgInfo>,
    /// Per-function SSA cache.
    ssa_cache: DashMap<FunctionId, SsaFunction>,
    /// Per-function contracts cache keyed by (FunctionId, version).
    contracts_cache: DashMap<(FunctionId, ContractVersion), ContractsReport>,
    /// Project-level call graph (computed once, shared).
    call_graph: OnceLock<ProjectCallGraph>,
    /// Change impact report (computed once, shared).
    change_impact: OnceLock<ChangeImpactReport>,
    /// Whether this is the first run (no prior `.bugbot/state.db`).
    ///
    /// When true, delta engines that require prior state (guard-removed,
    /// contract-regression) should suppress their findings because there
    /// is no baseline to compare against (PM-34 baseline policy).
    pub is_first_run: bool,
    /// Git base reference for baseline comparison (e.g. "HEAD", "main").
    /// Used by flow engines to create baseline worktrees for project-wide diffing.
    pub base_ref: String,
    /// Optional daemon client for routing IR queries through the daemon's
    /// QueryCache. When `is_available()` returns true, cache methods check
    /// the daemon first before falling back to local computation.
    daemon: Box<dyn DaemonClient>,
}

impl L2Context {
    /// Create a new L2Context with the provided data.
    ///
    /// All cache fields (CFG, DFG, SSA, contracts, call graph, change impact)
    /// are initialized empty and populated lazily on first access. The daemon
    /// client defaults to `NoDaemon` (local-only computation).
    pub fn new(
        project: PathBuf,
        language: Language,
        changed_files: Vec<PathBuf>,
        function_diff: FunctionDiff,
        baseline_contents: HashMap<PathBuf, String>,
        current_contents: HashMap<PathBuf, String>,
        ast_changes: HashMap<PathBuf, Vec<ASTChange>>,
    ) -> Self {
        Self {
            project,
            language,
            changed_files,
            function_diff,
            baseline_contents,
            current_contents,
            ast_changes,
            cfg_cache: DashMap::new(),
            dfg_cache: DashMap::new(),
            ssa_cache: DashMap::new(),
            contracts_cache: DashMap::new(),
            call_graph: OnceLock::new(),
            change_impact: OnceLock::new(),
            is_first_run: false,
            base_ref: String::from("HEAD"),
            daemon: Box::new(NoDaemon),
        }
    }

    /// Set whether this context represents a first-run analysis.
    ///
    /// When `is_first_run` is true, delta engines that require prior state
    /// (guard-removed, contract-regression) suppress their findings because
    /// there is no baseline to compare against (PM-34 baseline policy).
    pub fn with_first_run(mut self, is_first_run: bool) -> Self {
        self.is_first_run = is_first_run;
        self
    }

    /// Set the git base reference for baseline comparison.
    ///
    /// Used by flow engines (e.g. TldrDifferentialEngine) to create
    /// baseline worktrees for project-wide diffing of call graphs,
    /// dependencies, coupling, and cohesion.
    pub fn with_base_ref(mut self, base_ref: String) -> Self {
        self.base_ref = base_ref;
        self
    }

    /// Attach a daemon client to this context.
    ///
    /// When the daemon client reports `is_available() == true`, IR cache
    /// methods (cfg_for, dfg_for, ssa_for, call_graph) will check the daemon
    /// first before falling back to local computation. The daemon is also
    /// notified of `changed_files` for cache invalidation.
    pub fn with_daemon(mut self, daemon: Box<dyn DaemonClient>) -> Self {
        // Notify daemon of changed files so it can invalidate stale caches
        // before any queries are made in this analysis session.
        daemon.notify_changed_files(&self.changed_files);
        self.daemon = daemon;
        self
    }

    /// Check whether a daemon is available for this context.
    pub fn daemon_available(&self) -> bool {
        self.daemon.is_available()
    }

    /// Get a reference to the daemon client.
    pub fn daemon(&self) -> &dyn DaemonClient {
        self.daemon.as_ref()
    }

    /// Convenience accessor: functions whose bodies changed between revisions.
    pub fn changed_functions(&self) -> &[FunctionChange] {
        &self.function_diff.changed
    }

    /// Convenience accessor: functions present in current but not in baseline.
    pub fn inserted_functions(&self) -> &[InsertedFunction] {
        &self.function_diff.inserted
    }

    /// Convenience accessor: functions present in baseline but not in current.
    pub fn deleted_functions(&self) -> &[DeletedFunction] {
        &self.function_diff.deleted
    }

    /// Get or build the CFG for a function.
    ///
    /// Checks the local cache first, then queries the daemon if available.
    /// On miss, builds via `ir::build_cfg_for_function()` and stores the result.
    pub fn cfg_for(
        &self,
        file_contents: &str,
        function_id: &FunctionId,
        language: Language,
    ) -> anyhow::Result<dashmap::mapref::one::Ref<'_, FunctionId, CfgInfo>> {
        if let Some(entry) = self.cfg_cache.get(function_id) {
            return Ok(entry);
        }
        // Check daemon cache before computing locally
        if let Some(cached) = self.daemon.query_cfg(function_id) {
            self.cfg_cache.insert(function_id.clone(), cached);
            return Ok(self.cfg_cache.get(function_id).unwrap());
        }
        let cfg = super::ir::build_cfg_for_function(file_contents, function_id, language)?;
        self.cfg_cache.insert(function_id.clone(), cfg);
        Ok(self.cfg_cache.get(function_id).unwrap())
    }

    /// Get or build the DFG for a function.
    ///
    /// Checks the local cache first, then queries the daemon if available.
    /// On miss, builds via `ir::build_dfg_for_function()` and stores the result.
    pub fn dfg_for(
        &self,
        file_contents: &str,
        function_id: &FunctionId,
        language: Language,
    ) -> anyhow::Result<dashmap::mapref::one::Ref<'_, FunctionId, DfgInfo>> {
        if let Some(entry) = self.dfg_cache.get(function_id) {
            return Ok(entry);
        }
        // Check daemon cache before computing locally
        if let Some(cached) = self.daemon.query_dfg(function_id) {
            self.dfg_cache.insert(function_id.clone(), cached);
            return Ok(self.dfg_cache.get(function_id).unwrap());
        }
        let dfg = super::ir::build_dfg_for_function(file_contents, function_id, language)?;
        self.dfg_cache.insert(function_id.clone(), dfg);
        Ok(self.dfg_cache.get(function_id).unwrap())
    }

    /// Get or build the SSA for a function.
    ///
    /// Checks the local cache first, then queries the daemon if available.
    /// On miss, builds via `ir::build_ssa_for_function()` and stores the result.
    pub fn ssa_for(
        &self,
        file_contents: &str,
        function_id: &FunctionId,
        language: Language,
    ) -> anyhow::Result<dashmap::mapref::one::Ref<'_, FunctionId, SsaFunction>> {
        if let Some(entry) = self.ssa_cache.get(function_id) {
            return Ok(entry);
        }
        // Check daemon cache before computing locally
        if let Some(cached) = self.daemon.query_ssa(function_id) {
            self.ssa_cache.insert(function_id.clone(), cached);
            return Ok(self.ssa_cache.get(function_id).unwrap());
        }
        let ssa = super::ir::build_ssa_for_function(file_contents, function_id, language)?;
        self.ssa_cache.insert(function_id.clone(), ssa);
        Ok(self.ssa_cache.get(function_id).unwrap())
    }

    /// Get or insert a contracts report for a (function, version) pair.
    ///
    /// Checks the cache first; on miss, calls `build_fn` to produce the report
    /// and stores it.
    pub fn contracts_for(
        &self,
        function_id: &FunctionId,
        version: ContractVersion,
        build_fn: impl FnOnce() -> anyhow::Result<ContractsReport>,
    ) -> anyhow::Result<dashmap::mapref::one::Ref<'_, (FunctionId, ContractVersion), ContractsReport>>
    {
        let key = (function_id.clone(), version);
        if let Some(entry) = self.contracts_cache.get(&key) {
            return Ok(entry);
        }
        let report = build_fn()?;
        self.contracts_cache.insert(key.clone(), report);
        Ok(self.contracts_cache.get(&key).unwrap())
    }

    /// Get the cached call graph, if available.
    ///
    /// Checks the local OnceLock cache first. If empty and a daemon is
    /// available, queries the daemon for a cached call graph and stores
    /// it locally for subsequent accesses.
    pub fn call_graph(&self) -> Option<&ProjectCallGraph> {
        if let Some(cg) = self.call_graph.get() {
            return Some(cg);
        }
        // Check daemon cache before giving up
        if let Some(cached) = self.daemon.query_call_graph() {
            // OnceLock::set may fail if another thread set it concurrently
            let _ = self.call_graph.set(cached);
            return self.call_graph.get();
        }
        None
    }

    /// Set the call graph (can only be set once).
    pub fn set_call_graph(&self, cg: ProjectCallGraph) -> Result<(), ProjectCallGraph> {
        self.call_graph.set(cg)
    }

    /// Get the cached change impact report, if available.
    pub fn change_impact(&self) -> Option<&ChangeImpactReport> {
        self.change_impact.get()
    }

    /// Set the change impact report (can only be set once).
    ///
    /// Returns `Err` with the boxed report if the value was already set.
    pub fn set_change_impact(
        &self,
        report: ChangeImpactReport,
    ) -> Result<(), Box<ChangeImpactReport>> {
        self.change_impact.set(report).map_err(Box::new)
    }

    /// Create a minimal L2Context for testing purposes.
    #[cfg(test)]
    pub fn test_fixture() -> Self {
        Self::new(
            PathBuf::from("/tmp/test-project"),
            Language::Rust,
            vec![],
            FunctionDiff {
                changed: vec![],
                inserted: vec![],
                deleted: vec![],
            },
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_l2_context_new() {
        let ctx = L2Context::new(
            PathBuf::from("/projects/myapp"),
            Language::Python,
            vec![PathBuf::from("src/main.py")],
            FunctionDiff {
                changed: vec![],
                inserted: vec![],
                deleted: vec![],
            },
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        );

        assert_eq!(ctx.project, PathBuf::from("/projects/myapp"));
        assert_eq!(ctx.language, Language::Python);
        assert_eq!(ctx.changed_files.len(), 1);
        assert_eq!(ctx.changed_files[0], PathBuf::from("src/main.py"));
        assert!(ctx.changed_functions().is_empty());
        assert!(ctx.inserted_functions().is_empty());
        assert!(ctx.deleted_functions().is_empty());
        assert!(ctx.baseline_contents.is_empty());
        assert!(ctx.current_contents.is_empty());
        assert!(ctx.ast_changes.is_empty());
    }

    #[test]
    fn test_l2_context_test_fixture() {
        let ctx = L2Context::test_fixture();

        assert_eq!(ctx.project, PathBuf::from("/tmp/test-project"));
        assert_eq!(ctx.language, Language::Rust);
        assert!(ctx.changed_files.is_empty());
        assert!(ctx.changed_functions().is_empty());
        assert!(ctx.inserted_functions().is_empty());
        assert!(ctx.deleted_functions().is_empty());
        assert!(ctx.baseline_contents.is_empty());
        assert!(ctx.current_contents.is_empty());
        assert!(ctx.ast_changes.is_empty());
    }

    #[test]
    fn test_function_change_fields() {
        let change = FunctionChange {
            id: FunctionId::new("src/lib.rs", "compute", 1),
            name: "compute".to_string(),
            old_source: "fn compute() { 1 + 1 }".to_string(),
            new_source: "fn compute() { 2 + 2 }".to_string(),
        };

        assert_eq!(change.id.file, PathBuf::from("src/lib.rs"));
        assert_eq!(change.id.qualified_name, "compute");
        assert_eq!(change.name, "compute");
        assert!(change.old_source.contains("1 + 1"));
        assert!(change.new_source.contains("2 + 2"));
    }

    #[test]
    fn test_inserted_function_fields() {
        let inserted = InsertedFunction {
            id: FunctionId::new("src/new.rs", "fresh_func", 1),
            name: "fresh_func".to_string(),
            source: "fn fresh_func() -> bool { true }".to_string(),
        };

        assert_eq!(inserted.id.file, PathBuf::from("src/new.rs"));
        assert_eq!(inserted.id.qualified_name, "fresh_func");
        assert_eq!(inserted.name, "fresh_func");
        assert!(inserted.source.contains("true"));
    }

    #[test]
    fn test_deleted_function_fields() {
        let deleted = DeletedFunction {
            id: FunctionId::new("src/old.rs", "stale_func", 1),
            name: "stale_func".to_string(),
        };

        assert_eq!(deleted.id.file, PathBuf::from("src/old.rs"));
        assert_eq!(deleted.id.qualified_name, "stale_func");
        assert_eq!(deleted.name, "stale_func");
    }

    #[test]
    fn test_contract_version_eq() {
        assert_eq!(ContractVersion::Baseline, ContractVersion::Baseline);
        assert_eq!(ContractVersion::Current, ContractVersion::Current);
        assert_ne!(ContractVersion::Baseline, ContractVersion::Current);
        assert_ne!(ContractVersion::Current, ContractVersion::Baseline);
    }

    #[test]
    fn test_contract_version_hash_consistency() {
        use std::collections::HashSet;

        let mut set = HashSet::new();
        set.insert(ContractVersion::Baseline);
        set.insert(ContractVersion::Baseline); // duplicate
        set.insert(ContractVersion::Current);

        assert_eq!(
            set.len(),
            2,
            "HashSet should deduplicate identical variants"
        );
        assert!(set.contains(&ContractVersion::Baseline));
        assert!(set.contains(&ContractVersion::Current));
    }

    #[test]
    fn test_l2_context_with_data() {
        let file_a = PathBuf::from("src/alpha.rs");
        let file_b = PathBuf::from("src/beta.rs");
        let file_c = PathBuf::from("src/gamma.rs");

        let changed = vec![
            FunctionChange {
                id: FunctionId::new("src/alpha.rs", "do_alpha", 1),
                name: "do_alpha".to_string(),
                old_source: "fn do_alpha() {}".to_string(),
                new_source: "fn do_alpha() { todo!() }".to_string(),
            },
            FunctionChange {
                id: FunctionId::new("src/alpha.rs", "do_alpha2", 5),
                name: "do_alpha2".to_string(),
                old_source: "fn do_alpha2() {}".to_string(),
                new_source: "fn do_alpha2() { 42 }".to_string(),
            },
        ];

        let inserted = vec![InsertedFunction {
            id: FunctionId::new("src/beta.rs", "new_beta", 1),
            name: "new_beta".to_string(),
            source: "fn new_beta() -> u32 { 0 }".to_string(),
        }];

        let deleted = vec![DeletedFunction {
            id: FunctionId::new("src/gamma.rs", "old_gamma", 1),
            name: "old_gamma".to_string(),
        }];

        let mut baseline = HashMap::new();
        baseline.insert(file_a.clone(), "// alpha baseline".to_string());
        baseline.insert(file_c.clone(), "// gamma baseline".to_string());

        let mut current = HashMap::new();
        current.insert(file_a.clone(), "// alpha current".to_string());
        current.insert(file_b.clone(), "// beta current".to_string());

        let ctx = L2Context::new(
            PathBuf::from("/workspace"),
            Language::Rust,
            vec![file_a.clone(), file_b.clone(), file_c.clone()],
            FunctionDiff {
                changed,
                inserted,
                deleted,
            },
            baseline,
            current,
            HashMap::new(),
        );

        assert_eq!(ctx.changed_files.len(), 3);
        assert_eq!(ctx.changed_functions().len(), 2);
        assert_eq!(ctx.inserted_functions().len(), 1);
        assert_eq!(ctx.deleted_functions().len(), 1);
        assert_eq!(ctx.baseline_contents.len(), 2);
        assert_eq!(ctx.current_contents.len(), 2);

        // Verify specific data integrity
        assert_eq!(ctx.changed_functions()[0].name, "do_alpha");
        assert_eq!(ctx.changed_functions()[1].name, "do_alpha2");
        assert_eq!(ctx.inserted_functions()[0].name, "new_beta");
        assert_eq!(ctx.deleted_functions()[0].name, "old_gamma");

        assert_eq!(
            ctx.baseline_contents.get(&file_a).unwrap(),
            "// alpha baseline"
        );
        assert_eq!(
            ctx.current_contents.get(&file_b).unwrap(),
            "// beta current"
        );
    }

    #[test]
    fn test_function_change_clone() {
        let original = FunctionChange {
            id: FunctionId::new("src/lib.rs", "my_fn", 1),
            name: "my_fn".to_string(),
            old_source: "old".to_string(),
            new_source: "new".to_string(),
        };
        let cloned = original.clone();

        assert_eq!(cloned.id, original.id);
        assert_eq!(cloned.name, original.name);
        assert_eq!(cloned.old_source, original.old_source);
        assert_eq!(cloned.new_source, original.new_source);
    }

    #[test]
    fn test_inserted_function_clone() {
        let original = InsertedFunction {
            id: FunctionId::new("src/lib.rs", "ins_fn", 1),
            name: "ins_fn".to_string(),
            source: "fn ins_fn() {}".to_string(),
        };
        let cloned = original.clone();

        assert_eq!(cloned.id, original.id);
        assert_eq!(cloned.name, original.name);
        assert_eq!(cloned.source, original.source);
    }

    #[test]
    fn test_deleted_function_clone() {
        let original = DeletedFunction {
            id: FunctionId::new("src/lib.rs", "del_fn", 1),
            name: "del_fn".to_string(),
        };
        let cloned = original.clone();

        assert_eq!(cloned.id, original.id);
        assert_eq!(cloned.name, original.name);
    }

    #[test]
    fn test_contract_version_clone() {
        let v1 = ContractVersion::Baseline;
        let v2 = v1.clone();
        assert_eq!(v1, v2);

        let v3 = ContractVersion::Current;
        let v4 = v3.clone();
        assert_eq!(v3, v4);
    }

    // =========================================================================
    // Cache field tests
    // =========================================================================

    /// Simple Python function used for CFG/DFG/SSA cache tests.
    const PYTHON_ADD: &str = "def add(a, b):\n    return a + b\n";

    #[test]
    fn test_cfg_cache_miss_then_hit() {
        let ctx = L2Context::test_fixture();
        let fid = FunctionId::new("test.py", "add", 1);

        // First call: cache miss, builds the CFG.
        let result = ctx.cfg_for(PYTHON_ADD, &fid, Language::Python);
        assert!(
            result.is_ok(),
            "CFG build should succeed: {:?}",
            result.err()
        );
        let cfg = result.unwrap();
        assert_eq!(cfg.function, "add");
        drop(cfg);

        // Second call: cache hit (same result, no rebuild).
        let result2 = ctx.cfg_for(PYTHON_ADD, &fid, Language::Python);
        assert!(result2.is_ok());
        let cfg2 = result2.unwrap();
        assert_eq!(cfg2.function, "add");
    }

    #[test]
    fn test_dfg_cache_miss_then_hit() {
        let ctx = L2Context::test_fixture();
        let fid = FunctionId::new("test.py", "add", 1);

        // First call: cache miss, builds the DFG.
        let result = ctx.dfg_for(PYTHON_ADD, &fid, Language::Python);
        assert!(
            result.is_ok(),
            "DFG build should succeed: {:?}",
            result.err()
        );
        let dfg = result.unwrap();
        assert_eq!(dfg.function, "add");
        drop(dfg);

        // Second call: cache hit.
        let result2 = ctx.dfg_for(PYTHON_ADD, &fid, Language::Python);
        assert!(result2.is_ok());
        let dfg2 = result2.unwrap();
        assert_eq!(dfg2.function, "add");
    }

    #[test]
    fn test_ssa_cache_miss_then_hit() {
        let ctx = L2Context::test_fixture();
        let fid = FunctionId::new("test.py", "add", 1);

        // First call: cache miss, builds SSA.
        let result = ctx.ssa_for(PYTHON_ADD, &fid, Language::Python);
        assert!(
            result.is_ok(),
            "SSA build should succeed: {:?}",
            result.err()
        );
        let ssa = result.unwrap();
        assert_eq!(ssa.function, "add");
        drop(ssa);

        // Second call: cache hit.
        let result2 = ctx.ssa_for(PYTHON_ADD, &fid, Language::Python);
        assert!(result2.is_ok());
        let ssa2 = result2.unwrap();
        assert_eq!(ssa2.function, "add");
    }

    #[test]
    fn test_contracts_cache_stores_and_retrieves() {
        use crate::commands::contracts::types::ContractsReport;

        let ctx = L2Context::test_fixture();
        let fid = FunctionId::new("test.py", "add", 1);

        let report = ContractsReport {
            function: "add".to_string(),
            file: PathBuf::from("test.py"),
            preconditions: vec![],
            postconditions: vec![],
            invariants: vec![],
        };
        let report_clone = report.clone();

        // First call: cache miss, build_fn invoked.
        let result = ctx.contracts_for(&fid, ContractVersion::Baseline, || Ok(report));
        assert!(result.is_ok());
        let cached = result.unwrap();
        assert_eq!(cached.function, "add");
        drop(cached);

        // Second call: cache hit, build_fn is NOT invoked (would panic if called).
        let result2 = ctx.contracts_for(&fid, ContractVersion::Baseline, || {
            panic!("build_fn should not be called on cache hit");
        });
        assert!(result2.is_ok());
        assert_eq!(result2.unwrap().function, report_clone.function);
    }

    #[test]
    fn test_call_graph_set_and_get() {
        let ctx = L2Context::test_fixture();

        // Initially empty.
        assert!(ctx.call_graph().is_none());

        // Set the call graph.
        let cg = ProjectCallGraph::default();
        assert!(ctx.set_call_graph(cg).is_ok());

        // Now available.
        assert!(ctx.call_graph().is_some());
    }

    #[test]
    fn test_call_graph_double_set_fails() {
        let ctx = L2Context::test_fixture();

        let cg1 = ProjectCallGraph::default();
        assert!(ctx.set_call_graph(cg1).is_ok());

        // Second set returns Err (OnceLock semantics).
        let cg2 = ProjectCallGraph::default();
        assert!(ctx.set_call_graph(cg2).is_err());
    }

    #[test]
    fn test_change_impact_set_and_get() {
        let ctx = L2Context::test_fixture();

        // Initially empty.
        assert!(ctx.change_impact().is_none());

        let report = ChangeImpactReport {
            changed_files: vec![PathBuf::from("src/main.rs")],
            affected_tests: vec![],
            affected_test_functions: vec![],
            affected_functions: vec![],
            detection_method: "call_graph".to_string(),
            metadata: None,
            status: tldr_core::ChangeImpactStatus::Completed,
        };
        assert!(ctx.set_change_impact(report).is_ok());

        let stored = ctx.change_impact().unwrap();
        assert_eq!(stored.changed_files.len(), 1);
        assert_eq!(stored.detection_method, "call_graph");
    }

    #[test]
    fn test_cache_fields_independent() {
        let ctx = L2Context::test_fixture();
        let fid = FunctionId::new("test.py", "add", 1);

        // Build CFG for function.
        let cfg_result = ctx.cfg_for(PYTHON_ADD, &fid, Language::Python);
        assert!(cfg_result.is_ok());
        drop(cfg_result);

        // DFG cache for same function should still be empty (independent caches).
        assert!(
            ctx.dfg_cache.get(&fid).is_none(),
            "DFG cache should be empty when only CFG was built"
        );

        // Build DFG independently.
        let dfg_result = ctx.dfg_for(PYTHON_ADD, &fid, Language::Python);
        assert!(dfg_result.is_ok());
        drop(dfg_result);

        // Both caches now populated.
        assert!(ctx.cfg_cache.get(&fid).is_some());
        assert!(ctx.dfg_cache.get(&fid).is_some());
    }

    #[test]
    fn test_cfg_cache_different_functions() {
        let ctx = L2Context::test_fixture();

        let source = "def foo(x):\n    return x\n\ndef bar(y):\n    return y + 1\n";
        let fid_foo = FunctionId::new("multi.py", "foo", 1);
        let fid_bar = FunctionId::new("multi.py", "bar", 4);

        // Cache foo.
        let r1 = ctx.cfg_for(source, &fid_foo, Language::Python);
        assert!(r1.is_ok());
        let cfg_foo = r1.unwrap();
        assert_eq!(cfg_foo.function, "foo");
        drop(cfg_foo);

        // Cache bar independently.
        let r2 = ctx.cfg_for(source, &fid_bar, Language::Python);
        assert!(r2.is_ok());
        let cfg_bar = r2.unwrap();
        assert_eq!(cfg_bar.function, "bar");
        drop(cfg_bar);

        // Both cached independently.
        assert_eq!(ctx.cfg_cache.len(), 2);
        assert_eq!(ctx.cfg_cache.get(&fid_foo).unwrap().function, "foo");
        assert_eq!(ctx.cfg_cache.get(&fid_bar).unwrap().function, "bar");
    }

    // =========================================================================
    // PM-34: First-run field tests
    // =========================================================================

    #[test]
    fn test_l2_context_default_is_not_first_run() {
        let ctx = L2Context::test_fixture();
        assert!(
            !ctx.is_first_run,
            "Default L2Context should not be first run"
        );
    }

    #[test]
    fn test_l2_context_with_first_run_true() {
        let ctx = L2Context::test_fixture().with_first_run(true);
        assert!(
            ctx.is_first_run,
            "with_first_run(true) should set is_first_run"
        );
    }

    #[test]
    fn test_l2_context_with_first_run_false() {
        let ctx = L2Context::test_fixture().with_first_run(false);
        assert!(
            !ctx.is_first_run,
            "with_first_run(false) should unset is_first_run"
        );
    }

    #[test]
    fn test_l2_context_with_first_run_chainable() {
        // Verify that with_first_run is chainable (returns Self)
        let ctx = L2Context::new(
            PathBuf::from("/tmp/test"),
            Language::Python,
            vec![],
            FunctionDiff {
                changed: vec![],
                inserted: vec![],
                deleted: vec![],
            },
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
        .with_first_run(true);

        assert!(ctx.is_first_run);
        assert_eq!(ctx.project, PathBuf::from("/tmp/test"));
        assert_eq!(ctx.language, Language::Python);
    }

    // =========================================================================
    // Phase 8.4: Daemon integration tests
    // =========================================================================

    use super::super::daemon_client::DaemonClient;

    /// Mock daemon that provides a cached call graph.
    struct MockDaemonWithCallGraph {
        call_graph: ProjectCallGraph,
        notifications: std::sync::Mutex<Vec<Vec<PathBuf>>>,
    }

    impl MockDaemonWithCallGraph {
        fn new() -> Self {
            Self {
                call_graph: ProjectCallGraph::default(),
                notifications: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl DaemonClient for MockDaemonWithCallGraph {
        fn is_available(&self) -> bool {
            true
        }

        fn query_call_graph(&self) -> Option<ProjectCallGraph> {
            Some(self.call_graph.clone())
        }

        fn query_cfg(&self, _function_id: &FunctionId) -> Option<tldr_core::CfgInfo> {
            None
        }

        fn query_dfg(&self, _function_id: &FunctionId) -> Option<tldr_core::DfgInfo> {
            None
        }

        fn query_ssa(&self, _function_id: &FunctionId) -> Option<tldr_core::ssa::SsaFunction> {
            None
        }

        fn notify_changed_files(&self, changed_files: &[PathBuf]) {
            self.notifications
                .lock()
                .unwrap()
                .push(changed_files.to_vec());
        }
    }

    /// Mock daemon that is always unavailable (same as NoDaemon but verifiable).
    struct MockUnavailableDaemon;

    impl DaemonClient for MockUnavailableDaemon {
        fn is_available(&self) -> bool {
            false
        }
        fn query_call_graph(&self) -> Option<ProjectCallGraph> {
            None
        }
        fn query_cfg(&self, _fid: &FunctionId) -> Option<tldr_core::CfgInfo> {
            None
        }
        fn query_dfg(&self, _fid: &FunctionId) -> Option<tldr_core::DfgInfo> {
            None
        }
        fn query_ssa(&self, _fid: &FunctionId) -> Option<tldr_core::ssa::SsaFunction> {
            None
        }
        fn notify_changed_files(&self, _files: &[PathBuf]) {}
    }

    /// Default L2Context should have NoDaemon (not available).
    #[test]
    fn test_l2_context_default_daemon_not_available() {
        let ctx = L2Context::test_fixture();
        assert!(
            !ctx.daemon_available(),
            "Default L2Context should have NoDaemon (not available)"
        );
    }

    /// with_daemon should replace the default NoDaemon.
    #[test]
    fn test_l2_context_with_daemon_sets_available() {
        let ctx = L2Context::test_fixture().with_daemon(Box::new(MockDaemonWithCallGraph::new()));
        assert!(
            ctx.daemon_available(),
            "L2Context with mock daemon should report available"
        );
    }

    /// with_daemon should notify daemon of changed_files during construction.
    #[test]
    fn test_l2_context_with_daemon_notifies_changed_files() {
        use std::sync::Arc;

        // Shared notification tracker accessible after daemon is moved
        let notifications: Arc<std::sync::Mutex<Vec<Vec<PathBuf>>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));

        struct ArcTrackingDaemon {
            notified: Arc<std::sync::Mutex<Vec<Vec<PathBuf>>>>,
        }
        impl DaemonClient for ArcTrackingDaemon {
            fn is_available(&self) -> bool {
                true
            }
            fn query_call_graph(&self) -> Option<ProjectCallGraph> {
                None
            }
            fn query_cfg(&self, _fid: &FunctionId) -> Option<tldr_core::CfgInfo> {
                None
            }
            fn query_dfg(&self, _fid: &FunctionId) -> Option<tldr_core::DfgInfo> {
                None
            }
            fn query_ssa(&self, _fid: &FunctionId) -> Option<tldr_core::ssa::SsaFunction> {
                None
            }
            fn notify_changed_files(&self, files: &[PathBuf]) {
                self.notified.lock().unwrap().push(files.to_vec());
            }
        }

        let changed = vec![PathBuf::from("src/lib.rs"), PathBuf::from("src/main.rs")];
        let ctx = L2Context::new(
            PathBuf::from("/tmp/test"),
            Language::Rust,
            changed.clone(),
            FunctionDiff {
                changed: vec![],
                inserted: vec![],
                deleted: vec![],
            },
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        );

        let daemon = ArcTrackingDaemon {
            notified: Arc::clone(&notifications),
        };
        let ctx = ctx.with_daemon(Box::new(daemon));

        assert!(ctx.daemon_available());

        // Verify the daemon was notified of changed files during with_daemon
        let recorded = notifications.lock().unwrap();
        assert_eq!(
            recorded.len(),
            1,
            "with_daemon should have called notify_changed_files exactly once"
        );
        assert_eq!(
            recorded[0], changed,
            "notify_changed_files should receive the context's changed_files"
        );
    }

    /// When daemon is unavailable, call_graph() should return None.
    #[test]
    fn test_l2_context_daemon_not_available_call_graph_none() {
        let ctx = L2Context::test_fixture().with_daemon(Box::new(MockUnavailableDaemon));

        assert!(
            ctx.call_graph().is_none(),
            "Unavailable daemon should not provide call graph"
        );
    }

    /// When daemon is available and has a cached call graph, call_graph()
    /// should return it even though no local set_call_graph was called.
    #[test]
    fn test_l2_context_daemon_available_uses_cached_call_graph() {
        let ctx = L2Context::test_fixture().with_daemon(Box::new(MockDaemonWithCallGraph::new()));

        // No set_call_graph was called, but daemon provides one
        let cg = ctx.call_graph();
        assert!(
            cg.is_some(),
            "Available daemon should provide cached call graph"
        );
    }

    /// When daemon is not available, cfg_for falls back to local computation.
    #[test]
    fn test_l2_context_daemon_not_available_cfg_uses_local() {
        let ctx = L2Context::test_fixture().with_daemon(Box::new(MockUnavailableDaemon));
        let fid = FunctionId::new("test.py", "add", 1);

        // cfg_for should compute locally even without daemon
        let result = ctx.cfg_for(PYTHON_ADD, &fid, Language::Python);
        assert!(
            result.is_ok(),
            "cfg_for should fall back to local computation: {:?}",
            result.err()
        );
        let cfg = result.unwrap();
        assert_eq!(cfg.function, "add");
    }

    /// When daemon is not available, dfg_for falls back to local computation.
    #[test]
    fn test_l2_context_daemon_not_available_dfg_uses_local() {
        let ctx = L2Context::test_fixture().with_daemon(Box::new(MockUnavailableDaemon));
        let fid = FunctionId::new("test.py", "add", 1);

        let result = ctx.dfg_for(PYTHON_ADD, &fid, Language::Python);
        assert!(
            result.is_ok(),
            "dfg_for should fall back to local computation: {:?}",
            result.err()
        );
    }

    /// When daemon is not available, ssa_for falls back to local computation.
    #[test]
    fn test_l2_context_daemon_not_available_ssa_uses_local() {
        let ctx = L2Context::test_fixture().with_daemon(Box::new(MockUnavailableDaemon));
        let fid = FunctionId::new("test.py", "add", 1);

        let result = ctx.ssa_for(PYTHON_ADD, &fid, Language::Python);
        assert!(
            result.is_ok(),
            "ssa_for should fall back to local computation: {:?}",
            result.err()
        );
    }

    /// with_daemon is chainable with with_first_run.
    #[test]
    fn test_l2_context_with_daemon_chainable() {
        let ctx = L2Context::test_fixture()
            .with_first_run(true)
            .with_daemon(Box::new(MockDaemonWithCallGraph::new()));

        assert!(ctx.is_first_run);
        assert!(ctx.daemon_available());
    }

    /// daemon() accessor returns a reference to the daemon client.
    #[test]
    fn test_l2_context_daemon_accessor() {
        let ctx = L2Context::test_fixture();
        // Default daemon should not be available
        assert!(!ctx.daemon().is_available());

        let ctx2 = L2Context::test_fixture().with_daemon(Box::new(MockDaemonWithCallGraph::new()));
        assert!(ctx2.daemon().is_available());
    }

    /// Local call_graph set takes precedence over daemon query.
    #[test]
    fn test_l2_context_local_call_graph_takes_precedence() {
        let ctx = L2Context::test_fixture().with_daemon(Box::new(MockDaemonWithCallGraph::new()));

        // Set a local call graph
        let local_cg = ProjectCallGraph::default();
        assert!(ctx.set_call_graph(local_cg).is_ok());

        // call_graph() should return the local one (from OnceLock)
        let cg = ctx.call_graph();
        assert!(cg.is_some(), "Local call graph should take precedence");
    }
}
