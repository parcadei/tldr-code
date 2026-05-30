//! Multi-daemon registry (v0.3.0 VAL-003).
//!
//! Replaces v0.2.2's single-slot `daemon-active.json` with a multi-entry
//! `daemon-registry.json` file. Each entry records one running daemon; the
//! file always contains the union of all live daemons known to the user.
//!
//! # Concurrency (bounded registry write lock)
//!
//! The per-project flock at [`super::pid::try_acquire_lock`] (pid.rs:261,
//! `libc::flock(LOCK_EX | LOCK_NB)`) protects the SOCKET file, NOT the
//! registry. Two `daemon start` calls from DIFFERENT projects bypass that
//! flock and race read-modify-write the shared registry, so registry writes
//! are serialized with a small adjacent lock file and a bounded retry loop.
//!
//! # Migration from v0.2.x
//!
//! On first registry access, [`migrate_from_active_if_needed`] looks for the
//! legacy `daemon-active.json`; if present and its PID is alive, it is
//! converted into a registry entry and the legacy file is deleted. If the
//! PID is dead, the legacy file is also removed (stale record cleanup).

use std::fs::File;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One live-daemon record in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonRegistryEntry {
    /// Canonicalized project path the daemon was started with.
    pub project: PathBuf,
    /// PID of the daemon process. Validated via `kill(pid, 0)` on Unix.
    pub pid: u32,
    /// Path to the daemon's IPC socket (informational).
    pub socket: PathBuf,
    /// RFC3339 timestamp recorded at registration time.
    pub started_at: String,
}

/// On-disk registry shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonRegistry {
    /// All live daemons known at the time of read.
    pub daemons: Vec<DaemonRegistryEntry>,
}

const LOCK_RETRY_ATTEMPTS: usize = 50;
const LOCK_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(10);

/// Path to the daemon registry file.
///
/// Resolution order:
/// 1. `TLDR_DAEMON_REGISTRY_DIR` env override (used by tests for isolation).
/// 2. `<dirs::cache_dir()>/tldr/daemon-registry.json`.
/// 3. `./.cache/tldr/daemon-registry.json` fallback (mirrors `daemon_active`).
pub fn registry_file_path() -> PathBuf {
    if let Ok(dir) = std::env::var("TLDR_DAEMON_REGISTRY_DIR") {
        warn_registry_override_once();
        return PathBuf::from(dir).join("daemon-registry.json");
    }
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("tldr")
        .join("daemon-registry.json")
}

/// Emit a one-time stderr warning when the `TLDR_DAEMON_REGISTRY_DIR` override
/// is honored in a production build (W5). The override is a test-isolation
/// hook; in normal operation it silently redirects every daemon lookup, so a
/// caller (or attacker) who controls the environment could point clients at a
/// registry they own. Surfacing it once keeps the diagnostic visible without
/// spamming the many `registry_file_path` callers.
#[cfg(not(test))]
fn warn_registry_override_once() {
    use std::sync::Once;
    static WARN: Once = Once::new();
    WARN.call_once(|| {
        eprintln!(
            "warning: TLDR_DAEMON_REGISTRY_DIR is set — daemon registry lookups are \
             redirected to a non-default location. Unset it for normal operation."
        );
    });
}

#[cfg(test)]
fn warn_registry_override_once() {}

/// Create `dir` (if missing) and constrain it to owner-only access (`0700` on
/// unix). The registry records project paths and PIDs, so the directory must
/// not be world-traversable even under a permissive umask (e.g. `umask 000`
/// in CI/Docker), which would otherwise defeat the socket filename binding in
/// `ipc::connect_unix` (W3/W4).
fn ensure_secure_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Atomically write `registry` to [`registry_file_path`] via tmp + rename.
fn write_registry_atomic(registry: &DaemonRegistry) -> std::io::Result<()> {
    let path = registry_file_path();
    if let Some(parent) = path.parent() {
        ensure_secure_dir(parent)?;
    }
    let json = serde_json::to_string_pretty(registry).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    // Constrain the registry file to owner read/write before it is published
    // via rename — it leaks project paths and PIDs otherwise (W4). rename
    // preserves the mode, so the live file inherits 0600.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn registry_lock_path() -> PathBuf {
    registry_file_path().with_extension("json.lock")
}

fn with_registry_lock<T>(f: impl FnOnce() -> std::io::Result<T>) -> std::io::Result<T> {
    let lock_path = registry_lock_path();
    if let Some(parent) = lock_path.parent() {
        // The lock file is usually the first thing to create `<cache>/tldr`,
        // so secure the directory here too — not just in `write_registry_atomic`
        // (W3).
        ensure_secure_dir(parent)?;
    }

    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    lock_registry_file(&lock_file)?;

    let result = f();
    let unlock_result = unlock_registry_file(&lock_file);
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(err),
    }
}

#[cfg(unix)]
fn lock_registry_file(file: &File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;

    for _attempt in 0..LOCK_RETRY_ATTEMPTS {
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            return Ok(());
        }

        let err = std::io::Error::last_os_error();
        let raw = err.raw_os_error();
        if raw != Some(libc::EWOULDBLOCK) && raw != Some(libc::EAGAIN) {
            return Err(err);
        }
        std::thread::sleep(LOCK_RETRY_DELAY);
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::WouldBlock,
        "daemon registry lock contended after bounded retries",
    ))
}

#[cfg(unix)]
fn unlock_registry_file(file: &File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;

    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn lock_registry_file(_file: &File) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(unix))]
fn unlock_registry_file(_file: &File) -> std::io::Result<()> {
    Ok(())
}

/// Read the registry from disk, run one-shot v0.2.x migration if needed,
/// and prune dead-PID entries. The pruned-and-migrated registry is also
/// written back so subsequent reads observe a clean state.
///
/// Auxiliary state — a missing/corrupt file simply yields an empty registry.
pub fn read_registry() -> DaemonRegistry {
    migrate_from_active_if_needed();
    let path = registry_file_path();
    let mut registry = match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => DaemonRegistry::default(),
    };
    let original_len = registry.daemons.len();
    registry.daemons.retain(|d| is_pid_alive(d.pid));
    if registry.daemons.len() != original_len {
        // Pruned at least one stale entry — flush back. Best-effort.
        let _ = write_registry_atomic(&registry);
    }
    registry
}

/// Convenience: list of live entries (after pruning + migration).
pub fn live_entries() -> Vec<DaemonRegistryEntry> {
    read_registry().daemons
}

/// Look up a registry entry by canonicalized project path.
pub fn find_entry(project: &Path) -> Option<DaemonRegistryEntry> {
    let canon = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());
    read_registry()
        .daemons
        .into_iter()
        .find(|d| d.project == canon)
}

/// Read the registry from disk WITHOUT pruning dead-PID entries, migrating, or
/// writing back.
///
/// [`read_registry`] prunes dead entries, which is wrong for socket cleanup:
/// the stale-cleanup scenario is *precisely* when the daemon is dead, so a
/// pruning read would drop the very entry whose recorded socket path must be
/// removed — leaving a cross-TMPDIR orphan behind (W6). Callers that act on a
/// dead daemon's record (only `ipc::cleanup_socket`) must use this.
fn read_registry_unpruned() -> DaemonRegistry {
    let path = registry_file_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => DaemonRegistry::default(),
    }
}

/// Look up a registry entry by canonicalized project path WITHOUT pruning dead
/// PIDs. The returned entry's [`is_pid_alive`] status is the caller's to check.
/// See [`read_registry_unpruned`] for why cleanup must not prune.
pub fn find_entry_unpruned(project: &Path) -> Option<DaemonRegistryEntry> {
    let canon = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());
    read_registry_unpruned()
        .daemons
        .into_iter()
        .find(|d| d.project == canon)
}

/// Add (or replace) the registry entry for `project` while holding the
/// registry write lock.
pub fn add_entry(project: &Path, pid: u32, socket: &Path) -> std::io::Result<()> {
    let canon = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());

    with_registry_lock(|| {
        let mut registry = read_registry();
        registry.daemons.retain(|d| d.project != canon);
        registry.daemons.push(DaemonRegistryEntry {
            project: canon.clone(),
            pid,
            socket: socket.to_path_buf(),
            started_at: chrono::Utc::now().to_rfc3339(),
        });
        write_registry_atomic(&registry)
    })
}

/// Remove the registry entry for `project` while holding the registry write
/// lock.
pub fn remove_entry(project: &Path) -> std::io::Result<()> {
    let canon = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());

    with_registry_lock(|| {
        let mut registry = read_registry();
        let before = registry.daemons.len();
        registry.daemons.retain(|d| d.project != canon);
        if registry.daemons.len() == before {
            // Nothing to remove — caller's invariant satisfied.
            return Ok(());
        }
        write_registry_atomic(&registry)
    })
}

/// One-shot migration from v0.2.x `daemon-active.json`.
///
/// Triggered on first registry access. If a legacy daemon-active.json exists
/// and its PID is alive, append it as a registry entry. Either way, delete
/// the legacy file so subsequent registry reads do not re-trigger migration.
fn migrate_from_active_if_needed() {
    let registry_path = registry_file_path();
    if registry_path.exists() {
        return;
    }

    // Use the same parent dir as the registry for the legacy file lookup.
    // This matches the production layout (both files share `<cache>/tldr/`)
    // AND the test layout (both share the env-overridden directory).
    let active_path = match registry_path.parent() {
        Some(p) => p.join("daemon-active.json"),
        None => return,
    };
    if !active_path.exists() {
        return;
    }

    // Best-effort: read + validate + migrate. Failures collapse to "delete
    // the legacy file and move on" so the user is not stuck with the
    // legacy file blocking new registry creation.
    let migrated = match std::fs::read_to_string(&active_path) {
        Ok(content) => match serde_json::from_str::<super::daemon_active::DaemonActive>(&content) {
            Ok(active) if is_pid_alive(active.pid) => Some(DaemonRegistryEntry {
                project: active.project,
                pid: active.pid,
                socket: active.socket,
                started_at: chrono::Utc::now().to_rfc3339(),
            }),
            _ => None,
        },
        Err(_) => None,
    };

    if let Some(entry) = migrated {
        let registry = DaemonRegistry {
            daemons: vec![entry],
        };
        let _ = write_registry_atomic(&registry);
    }
    // Always delete the legacy file once migration has been attempted —
    // a dead-PID record is stale and should not block future registry
    // creation.
    let _ = std::fs::remove_file(&active_path);
}

/// Best-effort PID liveness check. Mirrors `daemon_active::is_pid_alive`.
#[cfg(unix)]
pub(crate) fn is_pid_alive(pid: u32) -> bool {
    let rc = unsafe { libc::kill(pid as i32, 0) };
    if rc == 0 {
        return true;
    }
    matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM)
    )
}

#[cfg(not(unix))]
pub(crate) fn is_pid_alive(_pid: u32) -> bool {
    true
}

/// Test-only support shared with sibling modules (e.g. `ipc`'s cleanup tests),
/// which must redirect the registry to a temp dir so they neither read nor
/// rewrite the developer's real `~/Library/Caches/tldr/daemon-registry.json`.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Serialize tests that mutate the process-global TLDR_DAEMON_REGISTRY_DIR
    /// env var. Without this, parallel tests stomp on each other's overrides
    /// and `add_entry` sees a NotFound when another thread has already
    /// removed the env var (registry dir resolves to a non-existent default).
    pub(crate) static REGISTRY_ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Helper: scope an env var override for the duration of a closure. The
    /// returned temp dir IS the registry directory for the closure's body.
    pub(crate) fn with_registry_dir<F: FnOnce(&Path)>(f: F) {
        // Hold the lock for the entire body so set_var / f / remove_var
        // run atomically with respect to other tests in this binary.
        let _guard = REGISTRY_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = TempDir::new().expect("tempdir");
        std::env::set_var("TLDR_DAEMON_REGISTRY_DIR", tmp.path());
        f(tmp.path());
        std::env::remove_var("TLDR_DAEMON_REGISTRY_DIR");
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::with_registry_dir;
    use super::*;

    #[test]
    fn registry_path_honors_env_override() {
        with_registry_dir(|dir| {
            let path = registry_file_path();
            assert_eq!(path, dir.join("daemon-registry.json"));
        });
    }

    #[test]
    fn read_registry_on_missing_file_returns_empty() {
        with_registry_dir(|_dir| {
            let r = read_registry();
            assert!(r.daemons.is_empty());
        });
    }

    #[cfg(unix)]
    #[test]
    fn registry_dir_and_file_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        with_registry_dir(|dir| {
            let project = dir.join("perms-proj");
            std::fs::create_dir_all(&project).unwrap();
            add_entry(&project, std::process::id(), &dir.join("perms.sock")).expect("add");

            let file = registry_file_path();
            let file_mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(file_mode, 0o600, "registry file must be owner-only (W4)");

            let dir_mode = std::fs::metadata(file.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dir_mode, 0o700, "registry dir must be owner-only (W3)");
        });
    }

    #[test]
    fn add_then_find_round_trips() {
        with_registry_dir(|dir| {
            let project = dir.join("proj");
            std::fs::create_dir_all(&project).unwrap();
            let socket = dir.join("proj.sock");
            add_entry(&project, std::process::id(), &socket).expect("add");
            let found = find_entry(&project).expect("entry should exist");
            assert_eq!(found.pid, std::process::id());
            assert_eq!(found.socket, socket);
        });
    }

    #[test]
    fn remove_entry_drops_record() {
        with_registry_dir(|dir| {
            let project = dir.join("proj-r");
            std::fs::create_dir_all(&project).unwrap();
            let socket = dir.join("proj-r.sock");
            add_entry(&project, std::process::id(), &socket).expect("add");
            remove_entry(&project).expect("remove");
            assert!(find_entry(&project).is_none());
        });
    }

    #[test]
    fn dead_pid_entries_are_pruned_on_read() {
        with_registry_dir(|dir| {
            let project = dir.join("proj-dead");
            std::fs::create_dir_all(&project).unwrap();
            // Spawn `true` and reap → PID is now definitely dead.
            let mut child = std::process::Command::new("true")
                .spawn()
                .expect("spawn true");
            let dead_pid = child.id();
            let _ = child.wait();
            // Inject a dead-pid entry directly via add_entry (which writes
            // the PID we hand it; the prune happens on subsequent reads).
            let socket = dir.join("proj-dead.sock");
            add_entry(&project, dead_pid, &socket).expect("add");
            let live = live_entries();
            assert!(
                live.iter().all(|d| d.pid != dead_pid),
                "dead PID entry should have been pruned on read"
            );
        });
    }
}
