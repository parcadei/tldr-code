//! Salsa-style incremental computation cache
//!
//! Implements query memoization with automatic invalidation based on input changes.
//! Uses DashMap for thread-safe concurrent access.
//!
//! # Design
//!
//! - Query results are keyed by `(query_name, args_hash)`
//! - Each entry tracks which inputs (files) it depends on
//! - When an input changes, all dependent queries are invalidated
//! - LRU eviction when cache exceeds max entries
//!
//! # Security Mitigations
//!
//! - TIGER-P2-01: Atomic writes with checksum validation for persistence
//! - Cache size limits to prevent OOM

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::SystemTime;

use dashmap::DashMap;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tldr_core::Language;

use super::error::DaemonResult;
use super::types::SalsaCacheStats;

// =============================================================================
// Constants
// =============================================================================

/// Default maximum number of cache entries
pub const DEFAULT_MAX_ENTRIES: usize = 10_000;

/// Default maximum cache size in bytes (512 MB)
pub const DEFAULT_MAX_BYTES: usize = 512 * 1024 * 1024;

/// Magic bytes for cache file validation
const CACHE_MAGIC: &[u8; 4] = b"TLDR";

/// Cache file version (header byte — distinct from `CACHE_SCHEMA_VERSION`,
/// which versions the JSON payload). Bumping this byte is reserved for
/// changes to the binary framing (magic / checksum layout / header order).
const CACHE_VERSION: u8 = 1;

/// Schema version for the JSON payload (`CacheFileData`).
///
/// v031-cluster-M3b: introduced alongside M3a's `language` field on
/// `QueryKey`. Bumped from the implicit v1 to v2 because pre-v0.3.1
/// caches lack the `language` field and cannot deserialize cleanly.
///
/// **Bumping policy:** increment whenever the JSON wire shape of
/// `CacheFileData`, `QueryKey`, or `CacheEntry` changes in a way that
/// would make older payloads fail to deserialize cleanly. Loading a
/// payload with a mismatched `schema_version` triggers graceful-discard
/// in `QueryCache::load_from_file`.
/// anonymous-callback-definitions-v1 bumped 2 → 3. The payload SHAPE did not change, but the
/// structure query now yields strictly more definitions (`kind: "call"`), and the daemon caches
/// structure results with NO dependencies (`daemon.rs` `insert(key, &val, vec![])`) and no
/// revision check on read — so a pre-upgrade entry stays valid forever and the new binary keeps
/// serving the old, callback-less map. A version that only tracks deserialization compatibility
/// cannot express "still parses, now wrong"; bumping it is the one mechanism that discards those.
pub const CACHE_SCHEMA_VERSION: u32 = 3;

// =============================================================================
// Core Types
// =============================================================================

/// Key for looking up cached query results
///
/// v031-cluster-M3a: extended with `language` to close the cross-language
/// cache contamination half of issue #27. Pre-fix, a Python query result
/// for function `foo` was served back when a TypeScript query for `foo`
/// arrived because keys collided on `(query_name, args_hash)` only. The
/// `language` field plus the `Hash`/`Eq` derive (which automatically picks
/// it up) place each language's results in disjoint cache slots.
///
/// **On-disk note:** `QueryKey` is serialized into the cache file via
/// `CacheFileData`. Adding `language` changes the JSON shape, so pre-v0.3.1
/// cache files cannot deserialize cleanly. Graceful-discard on schema
/// mismatch is owned by the sibling milestone v031-cluster-M3b
/// (`schema_version` header field). This struct change must ship in the
/// same release commit as M3b.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct QueryKey {
    /// Name of the query (e.g., "extract", "structure", "calls")
    pub query_name: String,
    /// Hash of the query arguments
    pub args_hash: u64,
    /// Language discriminator: prevents Python/TypeScript/etc. cache
    /// contamination when the same `(query_name, args_hash)` tuple is
    /// queried under different languages.
    pub language: Language,
}

impl QueryKey {
    /// Create a new query key
    pub fn new(query_name: impl Into<String>, args_hash: u64, language: Language) -> Self {
        Self {
            query_name: query_name.into(),
            args_hash,
            language,
        }
    }
}

/// Cached query result with metadata for invalidation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Serialized result value (JSON bytes)
    pub value: Vec<u8>,
    /// Revision number when this entry was created
    pub revision: u64,
    /// Hashes of inputs this query depends on (for invalidation tracking)
    pub input_hashes: Vec<u64>,
    /// When this entry was created
    #[serde(with = "system_time_serde")]
    pub created_at: SystemTime,
    /// Last access time (for LRU eviction)
    #[serde(with = "system_time_serde")]
    pub last_accessed: SystemTime,
}

impl CacheEntry {
    /// Create a new cache entry
    pub fn new(value: Vec<u8>, revision: u64, input_hashes: Vec<u64>) -> Self {
        let now = SystemTime::now();
        Self {
            value,
            revision,
            input_hashes,
            created_at: now,
            last_accessed: now,
        }
    }

    /// Estimated heap bytes used by this entry (value + input_hashes + overhead).
    pub fn estimated_bytes(&self) -> usize {
        self.value.len()
            + self.input_hashes.len() * std::mem::size_of::<u64>()
            + std::mem::size_of::<Self>()
    }
}

/// Salsa-style query cache with automatic invalidation
pub struct QueryCache {
    /// Cached query results: QueryKey -> CacheEntry
    entries: DashMap<QueryKey, CacheEntry>,
    /// Reverse index: input_hash -> Set of QueryKeys that depend on it
    dependents: DashMap<u64, HashSet<QueryKey>>,
    /// Global revision counter (incremented on any input change)
    revision: AtomicU64,
    /// Cache statistics
    stats: RwLock<SalsaCacheStats>,
    /// Maximum number of entries before eviction
    max_entries: usize,
    /// Maximum total bytes before eviction
    max_bytes: usize,
    /// Current total estimated bytes across all entries
    current_bytes: AtomicU64,
}

// =============================================================================
// QueryCache Implementation
// =============================================================================

impl QueryCache {
    /// Create a new query cache with the given max entries limit
    pub fn new(max_entries: usize) -> Self {
        Self::with_limits(max_entries, DEFAULT_MAX_BYTES)
    }

    /// Create a cache with explicit entry and byte limits
    pub fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: DashMap::new(),
            dependents: DashMap::new(),
            revision: AtomicU64::new(0),
            stats: RwLock::new(SalsaCacheStats::default()),
            max_entries,
            max_bytes,
            current_bytes: AtomicU64::new(0),
        }
    }

    /// Create a cache with default settings
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_MAX_ENTRIES)
    }

    /// Get a cached value if it exists and is valid
    ///
    /// Returns `None` if:
    /// - The key doesn't exist in cache
    /// - Deserialization fails
    pub fn get<T: DeserializeOwned>(&self, key: &QueryKey) -> Option<T> {
        if let Some(mut entry) = self.entries.get_mut(key) {
            // Update last accessed time
            entry.last_accessed = SystemTime::now();

            // Record hit
            if let Ok(mut stats) = self.stats.write() {
                stats.hits += 1;
            }

            // Try to deserialize the value
            match serde_json::from_slice(&entry.value) {
                Ok(value) => Some(value),
                Err(_) => {
                    // Corrupted entry - remove it
                    drop(entry);
                    self.entries.remove(key);
                    None
                }
            }
        } else {
            // Record miss
            if let Ok(mut stats) = self.stats.write() {
                stats.misses += 1;
            }
            None
        }
    }

    /// Insert a value into the cache
    ///
    /// The `input_hashes` are used for invalidation - when any of these
    /// inputs change, this entry will be invalidated.
    pub fn insert<T: Serialize>(&self, key: QueryKey, value: &T, input_hashes: Vec<u64>) {
        // Serialize the value
        let serialized = match serde_json::to_vec(value) {
            Ok(v) => v,
            Err(_) => return, // Can't serialize - skip caching
        };

        let revision = self.revision.load(Ordering::Acquire);
        let entry = CacheEntry::new(serialized, revision, input_hashes.clone());

        // Track dependencies for invalidation
        for &hash in &input_hashes {
            self.dependents.entry(hash).or_default().insert(key.clone());
        }

        // Track bytes: subtract old entry if replacing
        if let Some(old) = self.entries.get(&key) {
            self.current_bytes
                .fetch_sub(old.estimated_bytes() as u64, Ordering::Relaxed);
        }

        // Track bytes for new entry
        self.current_bytes
            .fetch_add(entry.estimated_bytes() as u64, Ordering::Relaxed);

        // Insert the entry
        self.entries.insert(key, entry);

        // Evict if over entry count OR byte limit
        self.maybe_evict();
    }

    /// Invalidate all cache entries that depend on the given input
    ///
    /// Returns the number of entries invalidated.
    pub fn invalidate_by_input(&self, input_hash: u64) -> usize {
        // Increment global revision
        self.revision.fetch_add(1, Ordering::Release);

        let mut invalidated = 0;

        // Remove all entries that depend on this input
        if let Some((_, keys)) = self.dependents.remove(&input_hash) {
            for key in keys {
                if let Some((_, entry)) = self.entries.remove(&key) {
                    self.current_bytes
                        .fetch_sub(entry.estimated_bytes() as u64, Ordering::Relaxed);
                    invalidated += 1;
                }
            }
        }

        // Update stats
        if let Ok(mut stats) = self.stats.write() {
            stats.invalidations += invalidated as u64;
        }

        invalidated
    }

    /// Invalidate a cache entry by key
    ///
    /// Returns true if an entry was removed.
    pub fn invalidate(&self, key: &QueryKey) -> bool {
        if let Some((_, entry)) = self.entries.remove(key) {
            // Track bytes removed
            self.current_bytes
                .fetch_sub(entry.estimated_bytes() as u64, Ordering::Relaxed);

            // Clean up dependent tracking
            for hash in entry.input_hashes {
                if let Some(mut deps) = self.dependents.get_mut(&hash) {
                    deps.remove(key);
                }
            }

            if let Ok(mut stats) = self.stats.write() {
                stats.invalidations += 1;
            }

            true
        } else {
            false
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> SalsaCacheStats {
        self.stats.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Get current number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get current revision number
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    /// Clear all cache entries
    pub fn clear(&self) {
        self.entries.clear();
        self.dependents.clear();
        self.revision.store(0, Ordering::Release);
        self.current_bytes.store(0, Ordering::Relaxed);

        if let Ok(mut stats) = self.stats.write() {
            *stats = SalsaCacheStats::default();
        }
    }

    /// Total estimated bytes currently used by cached entries
    pub fn total_bytes(&self) -> usize {
        self.current_bytes.load(Ordering::Relaxed) as usize
    }

    /// Evict oldest entries if cache exceeds entry count or byte limit
    fn maybe_evict(&self) {
        let over_entries = self.entries.len() > self.max_entries;
        let over_bytes = self.total_bytes() > self.max_bytes;

        if !over_entries && !over_bytes {
            return;
        }

        // Collect entries with their last access times and sizes
        let mut entries_by_time: Vec<(QueryKey, SystemTime, usize)> = self
            .entries
            .iter()
            .map(|e| {
                (
                    e.key().clone(),
                    e.value().last_accessed,
                    e.value().estimated_bytes(),
                )
            })
            .collect();

        // Sort by last accessed time (oldest first)
        entries_by_time.sort_by(|a, b| a.1.cmp(&b.1));

        // Evict oldest entries until we're under BOTH limits
        for (key, _, _) in entries_by_time {
            if self.entries.len() <= self.max_entries && self.total_bytes() <= self.max_bytes {
                break;
            }
            self.invalidate(&key);
        }
    }

    // =========================================================================
    // Persistence
    // =========================================================================

    /// Save cache to a file with atomic write and checksum validation
    ///
    /// TIGER-P2-01: Uses write-to-temp + rename pattern for atomic writes.
    pub fn save_to_file(&self, path: &Path) -> DaemonResult<()> {
        // Collect entries for serialization
        let entries: Vec<(QueryKey, CacheEntry)> = self
            .entries
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();

        let dependents: Vec<(u64, Vec<QueryKey>)> = self
            .dependents
            .iter()
            .map(|e| (*e.key(), e.value().iter().cloned().collect()))
            .collect();

        let stats = self.stats();
        let revision = self.revision();

        let cache_data = CacheFileData {
            // v031-cluster-M3b: stamp the current schema version so future
            // loaders can graceful-discard mismatched payloads.
            schema_version: CACHE_SCHEMA_VERSION,
            entries,
            dependents,
            stats,
            revision,
        };

        // Serialize to JSON
        let json = serde_json::to_vec(&cache_data)?;

        // Calculate checksum
        let checksum = calculate_checksum(&json);

        // Write to temp file first (atomic write pattern)
        let temp_path = path.with_extension("tmp");
        {
            let file = File::create(&temp_path)?;
            let mut writer = BufWriter::new(file);

            // Write header: magic + version + checksum
            writer.write_all(CACHE_MAGIC)?;
            writer.write_all(&[CACHE_VERSION])?;
            writer.write_all(&checksum.to_le_bytes())?;
            writer.write_all(&json)?;
            writer.flush()?;
        }

        // Atomic rename
        fs::rename(&temp_path, path)?;

        Ok(())
    }

    /// Load cache from a file with checksum and schema-version validation.
    ///
    /// **v031-cluster-M3b graceful-discard contract:** any failure mode
    /// — file-system error after open succeeds, bad magic, unsupported
    /// header version, checksum mismatch, JSON parse failure, or stale
    /// `schema_version` — is treated as a corrupt/legacy cache. The
    /// offending file is removed and a fresh empty `QueryCache` is
    /// returned. This is the load-bearing fix for users upgrading from
    /// v0.3.0 to v0.3.1: pre-v0.3.1 caches lack the `language` field on
    /// `QueryKey` (M3a) and would otherwise crash the daemon on every
    /// startup. Only a missing file (the path doesn't exist or can't be
    /// opened) is propagated as a real error to the caller.
    pub fn load_from_file(path: &Path) -> DaemonResult<Self> {
        let file = File::open(path)?;
        match Self::try_load_payload(&file) {
            Ok(cache_data) if cache_data.schema_version == CACHE_SCHEMA_VERSION => {
                Ok(Self::from_cache_data(cache_data))
            }
            Ok(stale) => {
                eprintln!(
                    "tldr-cli: cache schema mismatch on {} (found schema_version={}, expected {}); discarding and starting fresh",
                    path.display(),
                    stale.schema_version,
                    CACHE_SCHEMA_VERSION,
                );
                let _ = fs::remove_file(path);
                Ok(Self::with_defaults())
            }
            Err(reason) => {
                eprintln!(
                    "tldr-cli: cache file at {} could not be loaded ({}); discarding and starting fresh",
                    path.display(),
                    reason,
                );
                let _ = fs::remove_file(path);
                Ok(Self::with_defaults())
            }
        }
    }

    /// Internal: parse the on-disk format into a `CacheFileData`. Returns
    /// `Err(reason)` on any validation/parse failure so the caller can
    /// route to graceful-discard. Reasons are humane strings (no
    /// `DaemonError`) — they only feed the warning log.
    fn try_load_payload(file: &File) -> Result<CacheFileData, String> {
        let mut reader = BufReader::new(file);

        let mut magic = [0u8; 4];
        reader
            .read_exact(&mut magic)
            .map_err(|e| format!("read magic: {}", e))?;
        if &magic != CACHE_MAGIC {
            return Err("invalid cache file magic".to_string());
        }

        let mut version = [0u8; 1];
        reader
            .read_exact(&mut version)
            .map_err(|e| format!("read version: {}", e))?;
        if version[0] != CACHE_VERSION {
            return Err(format!("unsupported cache header version: {}", version[0]));
        }

        let mut checksum_bytes = [0u8; 8];
        reader
            .read_exact(&mut checksum_bytes)
            .map_err(|e| format!("read checksum: {}", e))?;
        let stored_checksum = u64::from_le_bytes(checksum_bytes);

        let mut data = Vec::new();
        reader
            .read_to_end(&mut data)
            .map_err(|e| format!("read payload: {}", e))?;

        let actual_checksum = calculate_checksum(&data);
        if stored_checksum != actual_checksum {
            return Err("cache file checksum mismatch".to_string());
        }

        serde_json::from_slice::<CacheFileData>(&data)
            .map_err(|e| format!("deserialize payload: {}", e))
    }

    /// Internal: rebuild a populated `QueryCache` from a validated
    /// `CacheFileData` payload.
    fn from_cache_data(cache_data: CacheFileData) -> Self {
        let cache = Self::with_defaults();

        let mut total_bytes: u64 = 0;
        for (key, entry) in cache_data.entries {
            total_bytes += entry.estimated_bytes() as u64;
            cache.entries.insert(key, entry);
        }
        cache.current_bytes.store(total_bytes, Ordering::Relaxed);

        for (hash, keys) in cache_data.dependents {
            cache.dependents.insert(hash, keys.into_iter().collect());
        }

        cache.revision.store(cache_data.revision, Ordering::Release);

        if let Ok(mut stats) = cache.stats.write() {
            *stats = cache_data.stats;
        }

        cache
    }
}

impl Default for QueryCache {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// =============================================================================
// Helper Types
// =============================================================================

/// Serializable cache file data.
///
/// **JSON wire format.** This struct is the on-disk JSON payload of the
/// Salsa cache. The `schema_version` header field (v031-cluster-M3b) is
/// checked on load: a mismatch triggers graceful-discard rather than a
/// daemon-crashing deserialize error. See `CACHE_SCHEMA_VERSION` for the
/// bumping policy.
///
/// Pre-v0.3.1 cache files lack both `schema_version` and `language` (on
/// `QueryKey`) and therefore fail the version check (or fail to
/// deserialize entirely) — both paths converge on graceful-discard.
#[derive(Serialize, Deserialize, Default)]
pub struct CacheFileData {
    /// Schema version of this payload. Set to `CACHE_SCHEMA_VERSION` on save;
    /// any other value on load triggers graceful-discard.
    #[serde(default)]
    pub schema_version: u32,
    /// Cached query results.
    pub entries: Vec<(QueryKey, CacheEntry)>,
    /// Reverse-index of input-hash → dependent query keys.
    pub dependents: Vec<(u64, Vec<QueryKey>)>,
    /// Cache statistics at the time of save.
    pub stats: SalsaCacheStats,
    /// Global revision counter at the time of save.
    pub revision: u64,
}

/// Calculate a checksum for data validation
fn calculate_checksum(data: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

/// Serde module for SystemTime
mod system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
        duration.as_secs().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + Duration::from_secs(secs))
    }
}

// =============================================================================
// Utility Functions
// =============================================================================

/// Hash arguments for cache key generation
pub fn hash_args<T: Hash>(args: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    args.hash(&mut hasher);
    hasher.finish()
}

/// Hash a file path for input tracking
pub fn hash_path(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_query_cache_new() {
        let cache = QueryCache::new(100);
        assert_eq!(cache.max_entries, 100);
        assert!(cache.is_empty());
        assert_eq!(cache.revision(), 0);
    }

    #[test]
    fn test_query_cache_insert_and_get() {
        let cache = QueryCache::new(100);
        let key = QueryKey::new("test", 12345, Language::Python);
        let value = vec!["hello", "world"];

        cache.insert(key.clone(), &value, vec![]);

        let result: Option<Vec<String>> = cache.get(&key);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), vec!["hello", "world"]);
    }

    #[test]
    fn test_query_cache_miss() {
        let cache = QueryCache::new(100);
        let key = QueryKey::new("nonexistent", 99999, Language::Python);

        let result: Option<String> = cache.get(&key);
        assert!(result.is_none());

        let stats = cache.stats();
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.hits, 0);
    }

    #[test]
    fn test_query_cache_hit_tracking() {
        let cache = QueryCache::new(100);
        let key = QueryKey::new("test", 12345, Language::Python);
        cache.insert(key.clone(), &"value", vec![]);

        // First get - hit
        let _: Option<String> = cache.get(&key);
        // Second get - hit
        let _: Option<String> = cache.get(&key);

        let stats = cache.stats();
        assert_eq!(stats.hits, 2);
    }

    #[test]
    fn test_query_cache_invalidate_by_input() {
        let cache = QueryCache::new(100);
        let input_hash = hash_path(Path::new("/test/file.rs"));

        // Insert entries that depend on the input
        let key1 = QueryKey::new("query1", 1, Language::Python);
        let key2 = QueryKey::new("query2", 2, Language::Python);
        let key3 = QueryKey::new("query3", 3, Language::Python); // No dependency

        cache.insert(key1.clone(), &"value1", vec![input_hash]);
        cache.insert(key2.clone(), &"value2", vec![input_hash]);
        cache.insert(key3.clone(), &"value3", vec![]);

        assert_eq!(cache.len(), 3);

        // Invalidate by input
        let invalidated = cache.invalidate_by_input(input_hash);
        assert_eq!(invalidated, 2);
        assert_eq!(cache.len(), 1);

        // key3 should still be accessible
        let result: Option<String> = cache.get(&key3);
        assert!(result.is_some());

        // key1 and key2 should be gone
        let result: Option<String> = cache.get(&key1);
        assert!(result.is_none());
    }

    #[test]
    fn test_query_cache_invalidation_stats() {
        let cache = QueryCache::new(100);
        let key = QueryKey::new("test", 1, Language::Python);
        cache.insert(key.clone(), &"value", vec![12345]);

        cache.invalidate_by_input(12345);

        let stats = cache.stats();
        assert_eq!(stats.invalidations, 1);
    }

    #[test]
    fn test_query_cache_clear() {
        let cache = QueryCache::new(100);

        // Insert some entries
        cache.insert(QueryKey::new("q1", 1, Language::Python), &"v1", vec![]);
        cache.insert(QueryKey::new("q2", 2, Language::Python), &"v2", vec![]);

        assert_eq!(cache.len(), 2);

        cache.clear();

        assert!(cache.is_empty());
        assert_eq!(cache.revision(), 0);
    }

    #[test]
    fn test_query_cache_lru_eviction() {
        let cache = QueryCache::new(3); // Max 3 entries

        // Insert 4 entries
        cache.insert(QueryKey::new("q1", 1, Language::Python), &"v1", vec![]);
        std::thread::sleep(std::time::Duration::from_millis(10));
        cache.insert(QueryKey::new("q2", 2, Language::Python), &"v2", vec![]);
        std::thread::sleep(std::time::Duration::from_millis(10));
        cache.insert(QueryKey::new("q3", 3, Language::Python), &"v3", vec![]);
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Access q1 to make it recently used
        let _: Option<String> = cache.get(&QueryKey::new("q1", 1, Language::Python));
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Insert q4 - should evict q2 (oldest accessed)
        cache.insert(QueryKey::new("q4", 4, Language::Python), &"v4", vec![]);

        assert!(cache.len() <= 3);

        // q1 should still exist (was accessed recently)
        let result: Option<String> = cache.get(&QueryKey::new("q1", 1, Language::Python));
        assert!(result.is_some());
    }

    #[test]
    fn test_query_cache_persistence() {
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("test_cache.bin");

        // Create and populate cache
        let cache = QueryCache::new(100);
        cache.insert(QueryKey::new("test", 12345, Language::Python), &"hello world", vec![1, 2, 3]);
        cache.insert(QueryKey::new("test2", 67890, Language::Python), &vec![1, 2, 3], vec![]);

        // Save to file
        cache.save_to_file(&cache_path).unwrap();

        // Load from file
        let loaded = QueryCache::load_from_file(&cache_path).unwrap();

        // Verify contents
        assert_eq!(loaded.len(), 2);

        let result: Option<String> = loaded.get(&QueryKey::new("test", 12345, Language::Python));
        assert_eq!(result, Some("hello world".to_string()));

        let result: Option<Vec<i32>> = loaded.get(&QueryKey::new("test2", 67890, Language::Python));
        assert_eq!(result, Some(vec![1, 2, 3]));
    }

    #[test]
    fn test_query_cache_persistence_checksum_validation() {
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("test_cache.bin");

        // Create and save cache
        let cache = QueryCache::new(100);
        cache.insert(QueryKey::new("test", 1, Language::Python), &"value", vec![]);
        cache.save_to_file(&cache_path).unwrap();

        // Corrupt the file
        let mut data = fs::read(&cache_path).unwrap();
        if data.len() > 20 {
            data[20] ^= 0xFF; // Flip some bits
        }
        fs::write(&cache_path, data).unwrap();

        // v031-cluster-M3b: a corrupted cache no longer surfaces a hard
        // error; the load path detects the checksum mismatch, removes the
        // offending file, logs a warning, and returns a fresh empty
        // cache. This is intentional — a daemon that panics every
        // startup because a single byte flipped on disk is worse for
        // users than a one-time cold-start cost.
        let loaded = QueryCache::load_from_file(&cache_path)
            .expect("checksum mismatch must be discarded gracefully");
        assert_eq!(loaded.len(), 0, "discarded cache must be empty");
        assert!(
            !cache_path.exists(),
            "corrupted cache file must be removed by graceful-discard path"
        );
    }

    #[test]
    fn test_hash_args() {
        let args1 = ("query", "/path/to/file.rs", 42);
        let args2 = ("query", "/path/to/file.rs", 42);
        let args3 = ("query", "/path/to/other.rs", 42);

        assert_eq!(hash_args(&args1), hash_args(&args2));
        assert_ne!(hash_args(&args1), hash_args(&args3));
    }

    #[test]
    fn test_hash_path() {
        let path1 = Path::new("/foo/bar.rs");
        let path2 = Path::new("/foo/bar.rs");
        let path3 = Path::new("/foo/baz.rs");

        assert_eq!(hash_path(path1), hash_path(path2));
        assert_ne!(hash_path(path1), hash_path(path3));
    }

    #[test]
    fn test_query_key_equality() {
        let key1 = QueryKey::new("test", 12345, Language::Python);
        let key2 = QueryKey::new("test", 12345, Language::Python);
        let key3 = QueryKey::new("test", 99999, Language::Python);
        let key4 = QueryKey::new("other", 12345, Language::Python);

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_ne!(key1, key4);
    }

    #[test]
    fn test_cache_entry_creation() {
        let entry = CacheEntry::new(vec![1, 2, 3], 5, vec![100, 200]);

        assert_eq!(entry.value, vec![1, 2, 3]);
        assert_eq!(entry.revision, 5);
        assert_eq!(entry.input_hashes, vec![100, 200]);
        assert!(entry.created_at <= SystemTime::now());
        assert!(entry.last_accessed <= SystemTime::now());
    }

    #[test]
    fn test_stats_hit_rate_calculation() {
        let cache = QueryCache::new(100);

        // No queries yet
        let stats = cache.stats();
        assert_eq!(stats.hit_rate(), 0.0);

        // Insert and query
        cache.insert(QueryKey::new("test", 1, Language::Python), &"value", vec![]);
        let _: Option<String> = cache.get(&QueryKey::new("test", 1, Language::Python)); // hit
        let _: Option<String> = cache.get(&QueryKey::new("test", 2, Language::Python)); // miss
        let _: Option<String> = cache.get(&QueryKey::new("test", 1, Language::Python)); // hit

        let stats = cache.stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        // hit_rate = 2 / 3 * 100 = 66.67
        assert!((stats.hit_rate() - 66.67).abs() < 0.1);
    }

    #[test]
    fn test_revision_increments_on_invalidation() {
        let cache = QueryCache::new(100);
        assert_eq!(cache.revision(), 0);

        cache.invalidate_by_input(12345);
        assert_eq!(cache.revision(), 1);

        cache.invalidate_by_input(67890);
        assert_eq!(cache.revision(), 2);
    }

    #[test]
    fn test_multiple_entries_same_input() {
        let cache = QueryCache::new(100);
        let shared_input = 12345u64;

        // Multiple queries depend on the same input
        cache.insert(QueryKey::new("q1", 1, Language::Python), &"v1", vec![shared_input]);
        cache.insert(QueryKey::new("q2", 2, Language::Python), &"v2", vec![shared_input]);
        cache.insert(QueryKey::new("q3", 3, Language::Python), &"v3", vec![shared_input]);

        assert_eq!(cache.len(), 3);

        // Invalidating the shared input should remove all three
        let count = cache.invalidate_by_input(shared_input);
        assert_eq!(count, 3);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_entry_with_multiple_inputs() {
        let cache = QueryCache::new(100);
        let input1 = 111u64;
        let input2 = 222u64;

        // Entry depends on multiple inputs
        cache.insert(QueryKey::new("q1", 1, Language::Python), &"v1", vec![input1, input2]);

        // Invalidating either input should remove the entry
        assert_eq!(cache.len(), 1);
        cache.invalidate_by_input(input1);
        assert!(cache.is_empty());
    }

    // =========================================================================
    // Memory-bounded cache tests
    // =========================================================================

    #[test]
    fn test_total_bytes_tracking() {
        let cache = QueryCache::new(100);
        assert_eq!(cache.total_bytes(), 0);

        // Insert a value and check bytes increased
        cache.insert(QueryKey::new("q1", 1, Language::Python), &"hello", vec![]);
        let bytes_after_one = cache.total_bytes();
        assert!(
            bytes_after_one > 0,
            "total_bytes should increase after insert"
        );

        // Insert another and check it increased further
        cache.insert(QueryKey::new("q2", 2, Language::Python), &"world", vec![]);
        let bytes_after_two = cache.total_bytes();
        assert!(
            bytes_after_two > bytes_after_one,
            "total_bytes should increase with more entries"
        );

        // Clear and check it resets
        cache.clear();
        assert_eq!(cache.total_bytes(), 0);
    }

    #[test]
    fn test_bytes_decrease_on_invalidate() {
        let cache = QueryCache::new(100);
        cache.insert(QueryKey::new("q1", 1, Language::Python), &"value1", vec![]);
        cache.insert(QueryKey::new("q2", 2, Language::Python), &"value2", vec![]);
        let bytes_before = cache.total_bytes();

        cache.invalidate(&QueryKey::new("q1", 1, Language::Python));
        let bytes_after = cache.total_bytes();
        assert!(
            bytes_after < bytes_before,
            "total_bytes should decrease after invalidation"
        );
    }

    #[test]
    fn test_bytes_decrease_on_invalidate_by_input() {
        let cache = QueryCache::new(100);
        let input_hash = 42u64;

        cache.insert(QueryKey::new("q1", 1, Language::Python), &"value1", vec![input_hash]);
        cache.insert(QueryKey::new("q2", 2, Language::Python), &"value2", vec![input_hash]);
        let bytes_before = cache.total_bytes();
        assert!(bytes_before > 0);

        cache.invalidate_by_input(input_hash);
        assert_eq!(
            cache.total_bytes(),
            0,
            "total_bytes should be 0 after all entries invalidated"
        );
    }

    #[test]
    fn test_byte_limit_eviction() {
        // Set a very small byte limit (1 KB)
        let cache = QueryCache::with_limits(10_000, 1024);

        // Insert entries until we exceed the byte limit
        // Each entry with a 200-byte payload
        let payload = "x".repeat(200);
        for i in 0..20 {
            cache.insert(QueryKey::new("q", i, Language::Python), &payload, vec![]);
        }

        // Cache should have evicted to stay under 1 KB
        assert!(
            cache.total_bytes() <= 1024,
            "total_bytes ({}) should be <= 1024 after eviction",
            cache.total_bytes()
        );
        assert!(
            cache.len() < 20,
            "entry count ({}) should be < 20 after byte-based eviction",
            cache.len()
        );
    }

    #[test]
    fn test_large_entry_evicts_many_small() {
        // 2 KB limit
        let cache = QueryCache::with_limits(10_000, 2048);

        // Insert 10 small entries (~50 bytes each)
        for i in 0..10 {
            cache.insert(QueryKey::new("small", i, Language::Python), &"tiny", vec![]);
        }
        let count_before = cache.len();
        assert_eq!(count_before, 10);

        // Insert one large entry (~1500 bytes)
        let big_payload = "x".repeat(1500);
        cache.insert(QueryKey::new("big", 0, Language::Python), &big_payload, vec![]);

        // Should have evicted some small entries to make room
        assert!(
            cache.total_bytes() <= 2048,
            "total_bytes ({}) should be <= 2048",
            cache.total_bytes()
        );
        // The big entry should still be present (most recently inserted)
        let result: Option<String> = cache.get(&QueryKey::new("big", 0, Language::Python));
        assert!(result.is_some(), "large entry should survive eviction");
    }

    #[test]
    fn test_byte_tracking_on_replace() {
        let cache = QueryCache::new(100);

        // Insert a small value
        cache.insert(QueryKey::new("q1", 1, Language::Python), &"small", vec![]);
        let bytes_small = cache.total_bytes();

        // Replace with a large value
        let big = "x".repeat(10_000);
        cache.insert(QueryKey::new("q1", 1, Language::Python), &big, vec![]);
        let bytes_big = cache.total_bytes();

        assert!(
            bytes_big > bytes_small,
            "bytes should increase when replacing small with large"
        );
        assert_eq!(cache.len(), 1, "should still be one entry after replace");
    }

    #[test]
    fn test_memory_bounded_cache_under_stress() {
        // 100 KB limit
        let cache = QueryCache::with_limits(10_000, 100 * 1024);

        // Insert 1000 entries with varied sizes
        for i in 0..1000u64 {
            let size = ((i % 10) + 1) as usize * 100; // 100 to 1000 bytes
            let payload = "x".repeat(size);
            cache.insert(QueryKey::new("stress", i, Language::Python), &payload, vec![]);
        }

        // Cache must respect byte limit
        assert!(
            cache.total_bytes() <= 100 * 1024,
            "total_bytes ({}) should be <= 102400 after stress test",
            cache.total_bytes()
        );

        // Most recent entries should be accessible
        let result: Option<String> = cache.get(&QueryKey::new("stress", 999, Language::Python));
        assert!(result.is_some(), "most recent entry should be cached");
    }

    #[test]
    fn test_estimated_bytes_accuracy() {
        let small = CacheEntry::new(vec![1, 2, 3], 0, vec![]);
        let large = CacheEntry::new(vec![0u8; 10_000], 0, vec![1, 2, 3]);

        assert!(small.estimated_bytes() < large.estimated_bytes());
        assert!(small.estimated_bytes() > 0);
        // Large entry should account for the 10K payload
        assert!(
            large.estimated_bytes() >= 10_000,
            "estimated_bytes ({}) should be >= payload size",
            large.estimated_bytes()
        );
    }

    #[test]
    fn test_default_max_bytes() {
        let cache = QueryCache::with_defaults();
        assert_eq!(cache.max_bytes, DEFAULT_MAX_BYTES);
        assert_eq!(cache.max_bytes, 512 * 1024 * 1024); // 512 MB
    }

    // =========================================================================
    // Property-based tests (proptest)
    // =========================================================================

    mod proptest_cache {
        use super::*;
        use proptest::prelude::*;

        /// Recompute total bytes by summing all entries — ground truth.
        fn recompute_bytes(cache: &QueryCache) -> usize {
            cache
                .entries
                .iter()
                .map(|e| e.value().estimated_bytes())
                .sum()
        }

        /// Arbitrary cache operation
        #[derive(Debug, Clone)]
        enum CacheOp {
            Insert {
                key_id: u8,
                payload_len: usize,
                input_hash: u64,
            },
            InvalidateByInput(u64),
            InvalidateByKey(u8),
            Clear,
        }

        fn arb_cache_op() -> impl Strategy<Value = CacheOp> {
            prop_oneof![
                (any::<u8>(), 0..2000usize, any::<u64>()).prop_map(|(k, p, h)| CacheOp::Insert {
                    key_id: k,
                    payload_len: p,
                    input_hash: h % 16, // cluster hashes for overlap
                }),
                (any::<u64>()).prop_map(|h| CacheOp::InvalidateByInput(h % 16)),
                (any::<u8>()).prop_map(CacheOp::InvalidateByKey),
                Just(CacheOp::Clear),
            ]
        }

        proptest! {
            /// Invariant: tracked bytes == sum of all entry sizes after any
            /// sequence of insert/invalidate/clear operations.
            #[test]
            fn bytes_tracking_consistent(ops in prop::collection::vec(arb_cache_op(), 1..150)) {
                let cache = QueryCache::with_limits(500, 10_000_000);

                for op in ops {
                    match op {
                        CacheOp::Insert { key_id, payload_len, input_hash } => {
                            let key = QueryKey::new("prop", key_id as u64, Language::Python);
                            let payload = vec![0u8; payload_len];
                            cache.insert(key, &payload, vec![input_hash]);
                        }
                        CacheOp::InvalidateByInput(hash) => {
                            cache.invalidate_by_input(hash);
                        }
                        CacheOp::InvalidateByKey(key_id) => {
                            let key = QueryKey::new("prop", key_id as u64, Language::Python);
                            cache.invalidate(&key);
                        }
                        CacheOp::Clear => {
                            cache.clear();
                        }
                    }
                }

                let tracked = cache.total_bytes();
                let actual = recompute_bytes(&cache);
                prop_assert_eq!(tracked, actual,
                    "tracked bytes ({}) != recomputed bytes ({})", tracked, actual);
            }

            /// Invariant: entry count never exceeds max_entries after operations.
            #[test]
            fn entry_count_bounded(ops in prop::collection::vec(arb_cache_op(), 1..200)) {
                let max = 50;
                let cache = QueryCache::with_limits(max, 10_000_000);

                for op in ops {
                    match op {
                        CacheOp::Insert { key_id, payload_len, input_hash } => {
                            let key = QueryKey::new("prop", key_id as u64, Language::Python);
                            let payload = vec![0u8; payload_len];
                            cache.insert(key, &payload, vec![input_hash]);
                        }
                        CacheOp::InvalidateByInput(hash) => {
                            cache.invalidate_by_input(hash);
                        }
                        CacheOp::InvalidateByKey(key_id) => {
                            let key = QueryKey::new("prop", key_id as u64, Language::Python);
                            cache.invalidate(&key);
                        }
                        CacheOp::Clear => {
                            cache.clear();
                        }
                    }
                }

                prop_assert!(cache.len() <= max,
                    "cache size {} exceeds max {}", cache.len(), max);
            }

            /// Invariant: total bytes never exceeds max_bytes after operations.
            #[test]
            fn byte_limit_bounded(ops in prop::collection::vec(arb_cache_op(), 1..200)) {
                let max_bytes = 50_000;
                let cache = QueryCache::with_limits(500, max_bytes);

                for op in ops {
                    match op {
                        CacheOp::Insert { key_id, payload_len, input_hash } => {
                            let key = QueryKey::new("prop", key_id as u64, Language::Python);
                            let payload = vec![0u8; payload_len];
                            cache.insert(key, &payload, vec![input_hash]);
                        }
                        CacheOp::InvalidateByInput(hash) => {
                            cache.invalidate_by_input(hash);
                        }
                        CacheOp::InvalidateByKey(key_id) => {
                            let key = QueryKey::new("prop", key_id as u64, Language::Python);
                            cache.invalidate(&key);
                        }
                        CacheOp::Clear => {
                            cache.clear();
                        }
                    }
                }

                prop_assert!(cache.total_bytes() <= max_bytes,
                    "total bytes {} exceeds max {}", cache.total_bytes(), max_bytes);
            }

            /// Invariant: after clear(), cache is empty and bytes are zero.
            #[test]
            fn clear_resets_everything(
                inserts in prop::collection::vec((any::<u8>(), 0..500usize), 1..50)
            ) {
                let cache = QueryCache::with_limits(500, 10_000_000);

                for (key_id, payload_len) in inserts {
                    let key = QueryKey::new("prop", key_id as u64, Language::Python);
                    cache.insert(key, &vec![0u8; payload_len], vec![]);
                }

                cache.clear();

                prop_assert_eq!(cache.len(), 0);
                prop_assert_eq!(cache.total_bytes(), 0);
                prop_assert_eq!(recompute_bytes(&cache), 0);
            }

            /// Invariant: inserting same key twice updates bytes correctly
            /// (no double-counting).
            #[test]
            fn replace_in_place_no_leak(
                sizes in prop::collection::vec(0..5000usize, 2..20)
            ) {
                let cache = QueryCache::with_limits(500, 10_000_000);
                let key = QueryKey::new("same", 42, Language::Python);

                for size in &sizes {
                    cache.insert(key.clone(), &vec![0u8; *size], vec![]);
                }

                // Only one entry should exist
                prop_assert_eq!(cache.len(), 1);
                // Tracked bytes should match actual
                let tracked = cache.total_bytes();
                let actual = recompute_bytes(&cache);
                prop_assert_eq!(tracked, actual);
            }
        }
    }
}
