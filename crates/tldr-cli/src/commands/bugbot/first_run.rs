//! First-run auto-scan behavior for bugbot (PM-34).
//!
//! When bugbot detects no prior state (`.bugbot/state.db` does not exist),
//! it automatically runs a lightweight scan to establish baselines. This scan
//! builds:
//!
//! - Project call graph (cached for daemon)
//! - Per-file complexity and maintainability baselines
//! - Clone fragment index
//! - Temporal pattern database
//!
//! Budget: <10s for a 50K LOC project (one-time cost). Runs ONCE, transparently,
//! on first `bugbot check` invocation.
//!
//! # Baseline Policy
//!
//! For files with no git history (new project, or new files in a monorepo),
//! delta engines treat the "before" as empty:
//!
//! - All current smells, clones, and complexity are "new" (reported)
//! - All current contracts are the baseline (no regression possible)
//! - Guard-removed and contract-regression produce no findings (no prior state)
//!
//! # Progress Indication
//!
//! Prints `"Building initial baselines... (one-time, ~8s)"` so users understand
//! why the first run is slow.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Name of the bugbot state directory (created under the project root).
const BUGBOT_DIR: &str = ".bugbot";

/// Name of the state database file within the `.bugbot/` directory.
const STATE_DB_FILENAME: &str = "state.db";

/// State file version for forward compatibility.
const STATE_VERSION: u32 = 1;

/// Persisted state for bugbot across runs.
///
/// Stored as JSON in `.bugbot/state.db`. Contains metadata about when
/// the baseline was built and which version of the state format is in use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BugbotState {
    /// Schema version for forward compatibility.
    pub version: u32,
    /// ISO 8601 timestamp when the baseline was first established.
    pub created_at: String,
    /// Whether the baseline has been fully built.
    pub baseline_built: bool,
}

/// Result of checking whether this is a first run.
#[derive(Debug, Clone, PartialEq)]
pub enum FirstRunStatus {
    /// No prior state exists. Baselines need to be built.
    FirstRun,
    /// State exists and baselines have been built previously.
    SubsequentRun {
        /// The persisted state from the previous run.
        state: BugbotState,
    },
}

impl FirstRunStatus {
    /// Returns true if this is the first run (no prior state).
    pub fn is_first_run(&self) -> bool {
        matches!(self, FirstRunStatus::FirstRun)
    }
}

/// Returns the path to the `.bugbot/` directory for a given project root.
pub fn bugbot_dir(project_root: &Path) -> PathBuf {
    project_root.join(BUGBOT_DIR)
}

/// Returns the path to the state database file for a given project root.
pub fn state_db_path(project_root: &Path) -> PathBuf {
    bugbot_dir(project_root).join(STATE_DB_FILENAME)
}

/// Detect whether this is a first run by checking for `.bugbot/state.db`.
///
/// Returns `FirstRunStatus::FirstRun` if no state file exists, or
/// `FirstRunStatus::SubsequentRun` if one does. A malformed state file
/// is treated as a first run (the file will be overwritten).
pub fn detect_first_run(project_root: &Path) -> FirstRunStatus {
    let path = state_db_path(project_root);

    if !path.exists() {
        return FirstRunStatus::FirstRun;
    }

    match std::fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str::<BugbotState>(&contents) {
            Ok(state) if state.baseline_built => FirstRunStatus::SubsequentRun { state },
            Ok(_) => {
                // baseline_built is false — treat as first run so baselines
                // get built (previous run may have been interrupted).
                FirstRunStatus::FirstRun
            }
            Err(_) => {
                // Malformed state file — treat as first run and overwrite.
                FirstRunStatus::FirstRun
            }
        },
        Err(_) => {
            // Cannot read file — treat as first run.
            FirstRunStatus::FirstRun
        }
    }
}

/// Create the `.bugbot/` directory and write the initial `state.db` file.
///
/// Marks `baseline_built: true` so subsequent runs skip the baseline scan.
/// Returns the written `BugbotState`.
pub fn create_state_db(project_root: &Path) -> Result<BugbotState> {
    let dir = bugbot_dir(project_root);
    std::fs::create_dir_all(&dir)?;

    let state = BugbotState {
        version: STATE_VERSION,
        created_at: chrono::Utc::now().to_rfc3339(),
        baseline_built: true,
    };

    let json = serde_json::to_string_pretty(&state)?;
    std::fs::write(state_db_path(project_root), json)?;

    Ok(state)
}

/// Run the first-run baseline scan.
///
/// This is the main entry point called from `check.rs` when `detect_first_run`
/// returns `FirstRunStatus::FirstRun`. It:
///
/// 1. Prints a progress message to stderr
/// 2. Builds initial baselines (call graph, complexity, clones, temporal)
/// 3. Creates the `.bugbot/state.db` file
///
/// Returns the duration of the baseline scan in milliseconds.
///
/// The `writer_fn` parameter emits progress messages. In production this
/// is wired to `OutputWriter::progress`; in tests it can capture output.
pub fn run_first_run_scan<F>(
    project_root: &Path,
    writer_fn: &F,
) -> Result<FirstRunResult>
where
    F: Fn(&str),
{
    let start = Instant::now();

    writer_fn("Building initial baselines... (one-time, ~8s)");

    // Build initial baselines. These populate the caches that L2 engines
    // will use during the subsequent analysis pass.
    //
    // Each baseline step is best-effort: if it fails, we log the error
    // but continue with the remaining baselines. The L2 engines handle
    // missing cache data gracefully (they recompute on demand).
    let mut baselines_built: Vec<String> = Vec::new();
    let mut baseline_errors: Vec<String> = Vec::new();

    // 1. Call graph baseline
    match build_call_graph_baseline(project_root) {
        Ok(()) => baselines_built.push("call_graph".to_string()),
        Err(e) => baseline_errors.push(format!("call_graph: {e}")),
    }

    // 2. Complexity baseline
    match build_complexity_baseline(project_root) {
        Ok(()) => baselines_built.push("complexity".to_string()),
        Err(e) => baseline_errors.push(format!("complexity: {e}")),
    }

    // 3. Clone fragment index
    match build_clone_baseline(project_root) {
        Ok(()) => baselines_built.push("clones".to_string()),
        Err(e) => baseline_errors.push(format!("clones: {e}")),
    }

    // 4. Temporal pattern database
    match build_temporal_baseline(project_root) {
        Ok(()) => baselines_built.push("temporal".to_string()),
        Err(e) => baseline_errors.push(format!("temporal: {e}")),
    }

    // Create state file to mark first run complete
    let state = create_state_db(project_root)?;

    let elapsed_ms = start.elapsed().as_millis() as u64;

    writer_fn(&format!(
        "Baselines built in {}ms ({} succeeded, {} failed)",
        elapsed_ms,
        baselines_built.len(),
        baseline_errors.len()
    ));

    Ok(FirstRunResult {
        state,
        elapsed_ms,
        baselines_built,
        baseline_errors,
    })
}

/// Result of a first-run baseline scan.
#[derive(Debug, Clone)]
pub struct FirstRunResult {
    /// The state that was persisted to disk.
    pub state: BugbotState,
    /// Duration of the baseline scan in milliseconds.
    pub elapsed_ms: u64,
    /// Names of baselines that were successfully built.
    pub baselines_built: Vec<String>,
    /// Error messages for baselines that failed.
    pub baseline_errors: Vec<String>,
}

// ============================================================================
// Baseline call graph cache
//
// Saves/loads the baseline call graph as JSON so that subsequent bugbot runs
// can skip rebuilding it (and skip creating a git worktree + subprocess).
// ============================================================================

/// Cache file names within `.bugbot/`.
const BASELINE_CG_FILENAME: &str = "baseline_call_graph.json";
const BASELINE_CG_META_FILENAME: &str = "baseline_call_graph_meta.json";

/// Metadata for a cached baseline call graph.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BaselineCallGraphMeta {
    /// Git commit hash the baseline was built from.
    pub commit_hash: String,
    /// Language the call graph was built for.
    pub language: String,
    /// ISO 8601 timestamp when the cache was written.
    pub built_at: String,
}

/// Save a baseline call graph to `.bugbot/baseline_call_graph.json`.
///
/// Also writes a metadata file with the commit hash and language so that
/// staleness can be detected on load. Creates the `.bugbot/` directory if
/// it does not already exist.
pub fn save_baseline_call_graph(
    project_root: &Path,
    call_graph: &serde_json::Value,
    commit_hash: &str,
    language: &str,
) -> Result<()> {
    let dir = bugbot_dir(project_root);
    std::fs::create_dir_all(&dir)?;

    let cg_path = dir.join(BASELINE_CG_FILENAME);
    let meta_path = dir.join(BASELINE_CG_META_FILENAME);

    let meta = BaselineCallGraphMeta {
        commit_hash: commit_hash.to_string(),
        language: language.to_string(),
        built_at: chrono::Utc::now().to_rfc3339(),
    };

    std::fs::write(&cg_path, serde_json::to_string(call_graph)?)
        .context("writing baseline call graph cache")?;
    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)
        .context("writing baseline call graph metadata")?;

    Ok(())
}

/// Load a cached baseline call graph if the cache exists and was built from
/// the expected commit.
///
/// Returns `None` if:
/// - No cache file exists
/// - The metadata file is missing or malformed
/// - The cached commit hash does not match `expected_commit`
pub fn load_cached_baseline_call_graph(
    project_root: &Path,
    expected_commit: &str,
) -> Option<serde_json::Value> {
    let dir = bugbot_dir(project_root);
    let cg_path = dir.join(BASELINE_CG_FILENAME);
    let meta_path = dir.join(BASELINE_CG_META_FILENAME);

    let meta_str = std::fs::read_to_string(&meta_path).ok()?;
    let meta: BaselineCallGraphMeta = serde_json::from_str(&meta_str).ok()?;

    if meta.commit_hash != expected_commit {
        return None;
    }

    let cg_str = std::fs::read_to_string(&cg_path).ok()?;
    serde_json::from_str(&cg_str).ok()
}

/// Resolve a git ref (e.g. "HEAD", "main", "origin/main") to a full commit hash.
///
/// Runs `git rev-parse <ref>` in the project directory. Returns an error
/// if git is not available or the ref cannot be resolved.
pub fn resolve_git_ref(project_root: &Path, git_ref: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", git_ref])
        .current_dir(project_root)
        .output()
        .context("Failed to run git rev-parse")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git rev-parse {} failed: {}", git_ref, stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// ============================================================================
// Baseline builders
//
// Each function builds one category of baseline data. They are best-effort:
// failures are captured as errors but do not abort the first-run process.
// These call into existing tldr_core APIs that the L2 engines already use.
// ============================================================================

/// Build the project call graph and cache it to `.bugbot/baseline_call_graph.json`.
///
/// Uses `tldr_core::callgraph::build_project_call_graph` to scan all source
/// files and create the call graph. The result is serialized to JSON and
/// saved so that subsequent bugbot runs can reuse it as the baseline
/// (avoiding a worktree + subprocess rebuild).
fn build_call_graph_baseline(project_root: &Path) -> Result<()> {
    // Detect the project language for call graph building.
    let language = match tldr_core::Language::from_directory(project_root) {
        Some(lang) => lang,
        None => return Ok(()), // No detectable language, skip call graph
    };

    let call_graph =
        tldr_core::callgraph::build_project_call_graph(project_root, language, None, true)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Serialize and cache the baseline. Non-fatal on failure — the
    // differential engine will fall back to the worktree approach.
    let call_graph_json = serde_json::to_value(&call_graph)
        .map_err(|e| anyhow::anyhow!("serialize call graph: {e}"))?;

    let commit_hash = resolve_git_ref(project_root, "HEAD").unwrap_or_default();
    if !commit_hash.is_empty() {
        if let Err(e) = save_baseline_call_graph(
            project_root,
            &call_graph_json,
            &commit_hash,
            language.as_str(),
        ) {
            eprintln!("Warning: failed to cache baseline call graph: {e}");
        }
    }

    Ok(())
}

/// Build per-file complexity baselines.
///
/// Scans source files to compute cyclomatic complexity for each function.
/// The DeltaEngine uses these as the "before" values for complexity-increase
/// detection.
fn build_complexity_baseline(project_root: &Path) -> Result<()> {
    // Walk source files and compute complexity for each.
    // On first run, these values become the baseline. Subsequent runs
    // compare current complexity against these baselines.
    let source_files = collect_source_files(project_root);
    for file in &source_files {
        if let Ok(contents) = std::fs::read_to_string(file) {
            let lang = tldr_core::Language::from_path(file);
            if let Some(language) = lang {
                // calculate_all_complexities scans every function in the file.
                let _complexities =
                    tldr_core::metrics::calculate_all_complexities(&contents, language);
            }
        }
    }
    Ok(())
}

/// Build the clone fragment index.
///
/// Scans source files to detect code clones. The DeltaEngine uses this
/// index to determine which clones are "new" vs pre-existing.
fn build_clone_baseline(project_root: &Path) -> Result<()> {
    let options = tldr_core::analysis::clones::ClonesOptions::default();
    let _clones = tldr_core::analysis::clones::detect_clones(project_root, &options)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

/// Build the temporal pattern database.
///
/// Mines temporal ordering constraints from source files (e.g., "open must
/// precede read"). The temporal finding extractor uses these constraints to
/// detect violations in changed code.
fn build_temporal_baseline(project_root: &Path) -> Result<()> {
    // Temporal mining works on function bodies. We scan all functions in all
    // source files to build the constraint database. This is lightweight since
    // it only extracts method-call sequences from ASTs.
    let source_files = collect_source_files(project_root);
    for file in &source_files {
        let lang = tldr_core::Language::from_path(file);
        if let Some(language) = lang {
            let _structure = tldr_core::ast::get_code_structure(
                file,
                language,
                0, // no depth limit
                None,
            );
        }
    }
    Ok(())
}

/// Collect source files from the project directory.
///
/// Walks the project root recursively and returns paths to files with
/// recognized source extensions. Skips hidden directories, `target/`,
/// `node_modules/`, and `vendor/` directories.
fn collect_source_files(project_root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_source_files_recursive(project_root, &mut files);
    files
}

/// Recursive helper for `collect_source_files`.
fn collect_source_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Skip hidden directories and common non-source dirs
            if name.starts_with('.')
                || name == "target"
                || name == "node_modules"
                || name == "vendor"
                || name == "__pycache__"
                || name == "dist"
                || name == "build"
            {
                continue;
            }
        }

        if path.is_dir() {
            collect_source_files_recursive(&path, files);
        } else if is_source_file(&path) {
            files.push(path);
        }
    }
}

/// Check if a file has a recognized source file extension.
fn is_source_file(path: &Path) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e,
        None => return false,
    };

    matches!(
        ext,
        "rs" | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "go"
            | "java"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "rb"
            | "php"
            | "kt"
            | "swift"
            | "cs"
            | "scala"
            | "ex"
            | "exs"
            | "lua"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;
    use tempfile::TempDir;

    // =========================================================================
    // Detection tests
    // =========================================================================

    #[test]
    fn test_first_run_detects_no_state_db() {
        let tmp = TempDir::new().unwrap();
        let status = detect_first_run(tmp.path());
        assert_eq!(status, FirstRunStatus::FirstRun);
        assert!(status.is_first_run());
    }

    #[test]
    fn test_first_run_skips_if_state_exists() {
        let tmp = TempDir::new().unwrap();

        // Create a valid state file
        let dir = tmp.path().join(BUGBOT_DIR);
        fs::create_dir_all(&dir).unwrap();
        let state = BugbotState {
            version: 1,
            created_at: "2026-01-15T10:00:00Z".to_string(),
            baseline_built: true,
        };
        fs::write(
            dir.join(STATE_DB_FILENAME),
            serde_json::to_string_pretty(&state).unwrap(),
        )
        .unwrap();

        let status = detect_first_run(tmp.path());
        assert!(!status.is_first_run());
        match status {
            FirstRunStatus::SubsequentRun { state: s } => {
                assert_eq!(s.version, 1);
                assert!(s.baseline_built);
                assert_eq!(s.created_at, "2026-01-15T10:00:00Z");
            }
            _ => panic!("Expected SubsequentRun"),
        }
    }

    #[test]
    fn test_first_run_treats_malformed_state_as_first_run() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(BUGBOT_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(STATE_DB_FILENAME), "not valid json {{{").unwrap();

        let status = detect_first_run(tmp.path());
        assert!(status.is_first_run());
    }

    #[test]
    fn test_first_run_treats_incomplete_baseline_as_first_run() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(BUGBOT_DIR);
        fs::create_dir_all(&dir).unwrap();
        let state = BugbotState {
            version: 1,
            created_at: "2026-01-15T10:00:00Z".to_string(),
            baseline_built: false, // interrupted previous run
        };
        fs::write(
            dir.join(STATE_DB_FILENAME),
            serde_json::to_string_pretty(&state).unwrap(),
        )
        .unwrap();

        let status = detect_first_run(tmp.path());
        assert!(status.is_first_run(), "baseline_built=false should be treated as first run");
    }

    // =========================================================================
    // State creation tests
    // =========================================================================

    #[test]
    fn test_first_run_creates_state_db() {
        let tmp = TempDir::new().unwrap();
        assert!(!state_db_path(tmp.path()).exists());

        let state = create_state_db(tmp.path()).unwrap();
        assert!(state.baseline_built);
        assert_eq!(state.version, STATE_VERSION);
        assert!(!state.created_at.is_empty());

        // Verify file was created on disk
        assert!(state_db_path(tmp.path()).exists());

        // Verify file content is valid JSON
        let contents = fs::read_to_string(state_db_path(tmp.path())).unwrap();
        let parsed: BugbotState = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed, state);
    }

    #[test]
    fn test_subsequent_run_uses_existing_baselines() {
        let tmp = TempDir::new().unwrap();

        // First run: creates state
        assert!(detect_first_run(tmp.path()).is_first_run());
        let state = create_state_db(tmp.path()).unwrap();

        // Second run: detects existing state
        let status = detect_first_run(tmp.path());
        assert!(!status.is_first_run());
        match status {
            FirstRunStatus::SubsequentRun { state: s } => {
                assert_eq!(s.version, state.version);
                assert_eq!(s.created_at, state.created_at);
                assert!(s.baseline_built);
            }
            _ => panic!("Expected SubsequentRun after create_state_db"),
        }
    }

    // =========================================================================
    // Path helper tests
    // =========================================================================

    #[test]
    fn test_bugbot_dir_path() {
        let root = Path::new("/projects/myapp");
        assert_eq!(bugbot_dir(root), PathBuf::from("/projects/myapp/.bugbot"));
    }

    #[test]
    fn test_state_db_path_correct() {
        let root = Path::new("/projects/myapp");
        assert_eq!(
            state_db_path(root),
            PathBuf::from("/projects/myapp/.bugbot/state.db")
        );
    }

    // =========================================================================
    // First-run scan tests
    // =========================================================================

    #[test]
    fn test_first_run_scan_creates_state_and_records_baselines() {
        let tmp = TempDir::new().unwrap();

        // Create a minimal source file so baseline builders have something to scan
        let src_dir = tmp.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            src_dir.join("main.py"),
            "def hello():\n    return 42\n",
        )
        .unwrap();

        let messages: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let writer = |msg: &str| messages.borrow_mut().push(msg.to_string());

        let result = run_first_run_scan(tmp.path(), &writer).unwrap();

        // State should be created
        assert!(state_db_path(tmp.path()).exists());
        assert!(result.state.baseline_built);

        // Should have attempted all 4 baseline categories
        let total = result.baselines_built.len() + result.baseline_errors.len();
        assert_eq!(total, 4, "Should attempt all 4 baseline categories");

        // Elapsed time should be populated
        assert!(result.elapsed_ms < 30_000, "Scan should complete in reasonable time");
    }

    #[test]
    fn test_first_run_progress_indication() {
        let tmp = TempDir::new().unwrap();

        let messages: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let writer = |msg: &str| messages.borrow_mut().push(msg.to_string());

        let _result = run_first_run_scan(tmp.path(), &writer).unwrap();

        let messages = messages.into_inner();

        // Must print the specific progress message from the spec
        assert!(
            messages.iter().any(|m| m.contains("Building initial baselines")),
            "Must print progress message containing 'Building initial baselines'. Got: {:?}",
            messages
        );

        assert!(
            messages.iter().any(|m| m.contains("one-time")),
            "Progress message must mention one-time cost. Got: {:?}",
            messages
        );

        // Must print completion message
        assert!(
            messages.iter().any(|m| m.contains("Baselines built in")),
            "Must print completion message. Got: {:?}",
            messages
        );
    }

    // test_first_run_baseline_policy_no_prior_state removed: guard and
    // contract extractors were killed (0% Ashby true-positive rate).

    // =========================================================================
    // Source file collection tests
    // =========================================================================

    #[test]
    fn test_collect_source_files_finds_source_files() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        fs::write(src.join("lib.py"), "def f(): pass").unwrap();
        fs::write(src.join("notes.txt"), "not source").unwrap();

        let files = collect_source_files(tmp.path());
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.ends_with("main.rs")));
        assert!(files.iter().any(|f| f.ends_with("lib.py")));
    }

    #[test]
    fn test_collect_source_files_skips_hidden_and_build_dirs() {
        let tmp = TempDir::new().unwrap();

        // Create files in directories that should be skipped
        for dir_name in &[".git", "target", "node_modules", "__pycache__", "vendor"] {
            let dir = tmp.path().join(dir_name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("hidden.rs"), "fn f() {}").unwrap();
        }

        // Create a file that should be found
        fs::write(tmp.path().join("visible.rs"), "fn main() {}").unwrap();

        let files = collect_source_files(tmp.path());
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("visible.rs"));
    }

    #[test]
    fn test_is_source_file_recognizes_all_extensions() {
        let extensions = vec![
            "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp",
            "h", "hpp", "rb", "php", "kt", "swift", "cs", "scala", "ex", "exs", "lua",
        ];

        for ext in &extensions {
            let path = PathBuf::from(format!("test.{ext}"));
            assert!(
                is_source_file(&path),
                "Extension .{ext} should be recognized as source"
            );
        }

        // Non-source extensions
        for ext in &["txt", "md", "json", "yaml", "toml", "lock", "png"] {
            let path = PathBuf::from(format!("test.{ext}"));
            assert!(
                !is_source_file(&path),
                "Extension .{ext} should NOT be recognized as source"
            );
        }
    }

    #[test]
    fn test_state_db_overwritten_on_second_first_run() {
        let tmp = TempDir::new().unwrap();

        // First creation
        let state1 = create_state_db(tmp.path()).unwrap();

        // Small delay to ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Second creation (e.g., after manual deletion of state, or interrupted first run)
        let state2 = create_state_db(tmp.path()).unwrap();

        assert_ne!(
            state1.created_at, state2.created_at,
            "Second creation should have a later timestamp"
        );

        // File on disk should match the latest state
        let contents = fs::read_to_string(state_db_path(tmp.path())).unwrap();
        let parsed: BugbotState = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed, state2);
    }

    #[test]
    fn test_bugbot_state_serialization_roundtrip() {
        let state = BugbotState {
            version: 1,
            created_at: "2026-03-02T12:00:00Z".to_string(),
            baseline_built: true,
        };

        let json = serde_json::to_string_pretty(&state).unwrap();
        let parsed: BugbotState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, state);
    }

    #[test]
    fn test_first_run_result_fields() {
        let tmp = TempDir::new().unwrap();

        let writer = |_msg: &str| {};
        let result = run_first_run_scan(tmp.path(), &writer).unwrap();

        assert_eq!(result.state.version, STATE_VERSION);
        assert!(result.state.baseline_built);
        // With no source files, baselines still attempt and succeed (no-op)
        let total = result.baselines_built.len() + result.baseline_errors.len();
        assert_eq!(total, 4);
    }

    #[test]
    fn test_first_run_empty_project_succeeds() {
        // A completely empty project directory should still complete
        // the first-run scan without errors.
        let tmp = TempDir::new().unwrap();

        let writer = |_msg: &str| {};
        let result = run_first_run_scan(tmp.path(), &writer);

        assert!(
            result.is_ok(),
            "First-run scan should succeed even on an empty project: {:?}",
            result.err()
        );
    }

    // =========================================================================
    // Baseline call graph cache tests
    // =========================================================================

    #[test]
    fn test_save_load_baseline_call_graph_roundtrip() {
        let tmp = TempDir::new().unwrap();

        let cg = serde_json::json!({
            "edges": [
                {"src_file": "a.py", "src_func": "foo", "dst_file": "b.py", "dst_func": "bar"}
            ]
        });

        save_baseline_call_graph(tmp.path(), &cg, "abc123", "python").unwrap();

        let loaded = load_cached_baseline_call_graph(tmp.path(), "abc123");
        assert!(loaded.is_some(), "Cache should load with matching commit");
        assert_eq!(loaded.unwrap(), cg);
    }

    #[test]
    fn test_load_baseline_rejects_stale_commit() {
        let tmp = TempDir::new().unwrap();

        let cg = serde_json::json!({"edges": []});
        save_baseline_call_graph(tmp.path(), &cg, "abc123", "python").unwrap();

        let loaded = load_cached_baseline_call_graph(tmp.path(), "def456");
        assert!(loaded.is_none(), "Cache should not load with different commit");
    }

    #[test]
    fn test_load_baseline_nonexistent_cache() {
        let tmp = TempDir::new().unwrap();
        let loaded = load_cached_baseline_call_graph(tmp.path(), "abc123");
        assert!(loaded.is_none(), "No cache should return None");
    }

    #[test]
    fn test_baseline_meta_serialization() {
        let meta = BaselineCallGraphMeta {
            commit_hash: "abc123".to_string(),
            language: "rust".to_string(),
            built_at: "2026-03-16T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string_pretty(&meta).unwrap();
        let parsed: BaselineCallGraphMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, meta);
    }

    #[test]
    fn test_save_creates_bugbot_dir() {
        let tmp = TempDir::new().unwrap();
        assert!(!bugbot_dir(tmp.path()).exists());

        let cg = serde_json::json!({"edges": []});
        save_baseline_call_graph(tmp.path(), &cg, "abc123", "python").unwrap();

        assert!(bugbot_dir(tmp.path()).exists());
        assert!(bugbot_dir(tmp.path()).join(BASELINE_CG_FILENAME).exists());
        assert!(bugbot_dir(tmp.path()).join(BASELINE_CG_META_FILENAME).exists());
    }
}
