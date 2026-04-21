//! L1 in-process cache for MCP tool results
//!
//! Provides a TTL-based, bounded cache that sits in front of tool execution.
//! The cache is keyed on `tool_name:args_json` and stores `ToolsCallResult` values.
//!
//! Design constraints:
//! - Single-threaded server (blocking stdio loop), so `RefCell` is sufficient
//! - `ToolsCallResult` derives `Clone`, so cached values can be returned by clone
//! - TTL-based expiration prevents stale results for filesystem-dependent tools
//! - Max entries bound prevents unbounded memory growth

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::protocol::ToolsCallResult;

/// A cached tool result with insertion timestamp for TTL expiration.
struct CacheEntry {
    result: ToolsCallResult,
    inserted_at: Instant,
}

/// L1 in-process cache for MCP tool results.
///
/// Stores tool results keyed by a deterministic string derived from
/// the tool name and its JSON arguments. Entries expire after `ttl`
/// and the cache is bounded to `max_entries` to prevent unbounded growth.
pub struct L1Cache {
    entries: HashMap<String, CacheEntry>,
    ttl: Duration,
    max_entries: usize,
}

impl L1Cache {
    /// Create a new cache with the given TTL and maximum entry count.
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            ttl,
            max_entries,
        }
    }

    /// Look up a cached result by key.
    ///
    /// Returns `None` if the key is not present or the entry has expired.
    /// Expired entries are not removed here — eviction happens on insert.
    pub fn get(&self, key: &str) -> Option<&ToolsCallResult> {
        self.entries.get(key).and_then(|entry| {
            if entry.inserted_at.elapsed() < self.ttl {
                Some(&entry.result)
            } else {
                None
            }
        })
    }

    /// Insert a tool result into the cache.
    ///
    /// If the cache is at capacity, the oldest entry (by insertion time)
    /// is evicted before inserting the new one.
    pub fn insert(&mut self, key: String, result: ToolsCallResult) {
        if self.entries.len() >= self.max_entries {
            // Evict the oldest entry (earliest inserted_at)
            if let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.inserted_at)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest_key);
            }
        }
        self.entries.insert(
            key,
            CacheEntry {
                result,
                inserted_at: Instant::now(),
            },
        );
    }

    /// Return the number of entries currently in the cache.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Remove a specific key from the cache.
    pub fn invalidate(&mut self, key: &str) {
        self.entries.remove(key);
    }

    /// Remove all entries from the cache.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Build a deterministic cache key from a tool name and its JSON arguments.
    ///
    /// The key format is `tool_name:sorted_args_json` where object keys are
    /// recursively sorted to ensure `{"a":1,"b":2}` and `{"b":2,"a":1}` produce
    /// the same cache key.
    pub fn cache_key(tool_name: &str, args: &serde_json::Value) -> String {
        let sorted = Self::sort_json_keys(args);
        format!("{}:{}", tool_name, sorted)
    }

    /// Recursively sort all object keys in a JSON value.
    ///
    /// Arrays preserve element order; only object keys are sorted.
    fn sort_json_keys(value: &serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match value {
            Value::Object(map) => {
                let mut sorted: serde_json::Map<String, Value> = serde_json::Map::new();
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                for key in keys {
                    sorted.insert(key.clone(), Self::sort_json_keys(&map[key]));
                }
                Value::Object(sorted)
            }
            Value::Array(arr) => {
                Value::Array(arr.iter().map(Self::sort_json_keys).collect())
            }
            other => other.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::thread;
    use std::time::Duration;

    /// Helper: create a simple successful ToolsCallResult for testing.
    fn make_result(text: &str) -> ToolsCallResult {
        ToolsCallResult::text(text)
    }

    /// Helper: assert two ToolsCallResult values are semantically equal.
    ///
    /// Compares the content text and is_error flag since ToolsCallResult
    /// does not derive PartialEq (it only needs Serialize + Clone for production).
    fn assert_results_eq(a: &ToolsCallResult, b: &ToolsCallResult) {
        assert_eq!(a.is_error, b.is_error, "is_error mismatch");
        assert_eq!(a.content.len(), b.content.len(), "content length mismatch");
        for (ai, bi) in a.content.iter().zip(b.content.iter()) {
            assert_eq!(ai.content_type, bi.content_type, "content_type mismatch");
            assert_eq!(ai.text, bi.text, "text mismatch");
        }
    }

    // -----------------------------------------------------------------------
    // (a) Cache hit: insert a result, get with same key -> returns Some
    // -----------------------------------------------------------------------
    #[test]
    fn test_cache_hit_returns_stored_result() {
        let mut cache = L1Cache::new(Duration::from_secs(60), 100);
        let key = L1Cache::cache_key("tldr_tree", &json!({"path": "/src"}));
        let result = make_result("tree output");

        cache.insert(key.clone(), result.clone());

        let cached = cache.get(&key);
        assert!(cached.is_some(), "expected cache hit, got miss");
        assert_results_eq(cached.unwrap(), &result);
    }

    // -----------------------------------------------------------------------
    // (b) Cache miss on unknown key: get with unknown key -> returns None
    // -----------------------------------------------------------------------
    #[test]
    fn test_cache_miss_on_unknown_key() {
        let cache = L1Cache::new(Duration::from_secs(60), 100);
        let result = cache.get("nonexistent_key");
        assert!(result.is_none(), "expected cache miss for unknown key");
    }

    // -----------------------------------------------------------------------
    // (c) Cache miss on different args: insert with args A, get with args B -> None
    // -----------------------------------------------------------------------
    #[test]
    fn test_cache_miss_on_different_args() {
        let mut cache = L1Cache::new(Duration::from_secs(60), 100);
        let key_a = L1Cache::cache_key("tldr_tree", &json!({"path": "/src"}));
        let key_b = L1Cache::cache_key("tldr_tree", &json!({"path": "/tests"}));

        cache.insert(key_a, make_result("src tree"));

        let cached = cache.get(&key_b);
        assert!(cached.is_none(), "expected cache miss for different args");
    }

    // -----------------------------------------------------------------------
    // (d) TTL expiration: insert, wait past TTL, get -> None
    // -----------------------------------------------------------------------
    #[test]
    fn test_ttl_expiration_returns_none() {
        let mut cache = L1Cache::new(Duration::from_millis(1), 100);
        let key = L1Cache::cache_key("tldr_structure", &json!({"path": "."}));

        cache.insert(key.clone(), make_result("structure output"));

        // Sleep past the TTL
        thread::sleep(Duration::from_millis(10));

        let cached = cache.get(&key);
        assert!(cached.is_none(), "expected cache miss after TTL expiration");
    }

    // -----------------------------------------------------------------------
    // (e) TTL fresh: insert, get immediately -> Some (TTL not yet expired)
    // -----------------------------------------------------------------------
    #[test]
    fn test_ttl_fresh_returns_some() {
        let mut cache = L1Cache::new(Duration::from_secs(60), 100);
        let key = L1Cache::cache_key("tldr_calls", &json!({"path": "/src", "language": "rust"}));
        let result = make_result("call graph output");

        cache.insert(key.clone(), result.clone());

        let cached = cache.get(&key);
        assert!(cached.is_some(), "expected cache hit within TTL window");
        assert_results_eq(cached.unwrap(), &result);
    }

    // -----------------------------------------------------------------------
    // (f) Max entries eviction: insert max_entries + 1 -> oldest evicted
    // -----------------------------------------------------------------------
    #[test]
    fn test_max_entries_eviction() {
        let max = 3;
        let mut cache = L1Cache::new(Duration::from_secs(60), max);

        // Insert max_entries items
        for i in 0..max {
            let key = format!("key_{}", i);
            cache.insert(key, make_result(&format!("result_{}", i)));
        }
        assert_eq!(cache.len(), max, "cache should be at capacity");

        // Insert one more -- should evict the oldest (key_0)
        cache.insert("key_3".to_string(), make_result("result_3"));

        assert_eq!(cache.len(), max, "cache should still be at capacity after eviction");
        assert!(
            cache.get("key_0").is_none(),
            "oldest entry (key_0) should have been evicted"
        );
        assert!(
            cache.get("key_3").is_some(),
            "newest entry (key_3) should be present"
        );
    }

    // -----------------------------------------------------------------------
    // (g) Invalidation: insert, invalidate key, get -> None
    // -----------------------------------------------------------------------
    #[test]
    fn test_invalidation_removes_entry() {
        let mut cache = L1Cache::new(Duration::from_secs(60), 100);
        let key = L1Cache::cache_key("tldr_dead", &json!({"path": "/src", "language": "python"}));

        cache.insert(key.clone(), make_result("dead code output"));
        cache.invalidate(&key);

        let cached = cache.get(&key);
        assert!(cached.is_none(), "expected cache miss after invalidation");
    }

    // -----------------------------------------------------------------------
    // (h) Clear: insert items, clear, get -> None
    // -----------------------------------------------------------------------
    #[test]
    fn test_clear_removes_all_entries() {
        let mut cache = L1Cache::new(Duration::from_secs(60), 100);

        cache.insert("key_a".to_string(), make_result("result_a"));
        cache.insert("key_b".to_string(), make_result("result_b"));
        cache.insert("key_c".to_string(), make_result("result_c"));

        assert_eq!(cache.len(), 3, "precondition: cache should have 3 entries");

        cache.clear();

        assert_eq!(cache.len(), 0, "cache should be empty after clear");
        assert!(cache.get("key_a").is_none(), "key_a should be gone after clear");
        assert!(cache.get("key_b").is_none(), "key_b should be gone after clear");
        assert!(cache.get("key_c").is_none(), "key_c should be gone after clear");
    }

    // -----------------------------------------------------------------------
    // (i) Cache key determinism: same tool + same args -> same key
    // -----------------------------------------------------------------------
    #[test]
    fn test_cache_key_determinism() {
        let args = json!({"path": "/src", "language": "rust"});

        let key1 = L1Cache::cache_key("tldr_structure", &args);
        let key2 = L1Cache::cache_key("tldr_structure", &args);

        assert_eq!(key1, key2, "same tool + same args must produce the same cache key");
    }

    // -----------------------------------------------------------------------
    // (j) Cache key differentiation: different tool OR different args -> different keys
    // -----------------------------------------------------------------------
    #[test]
    fn test_cache_key_differentiation_by_tool_name() {
        let args = json!({"path": "/src"});

        let key1 = L1Cache::cache_key("tldr_tree", &args);
        let key2 = L1Cache::cache_key("tldr_structure", &args);

        assert_ne!(key1, key2, "different tool names must produce different cache keys");
    }

    #[test]
    fn test_cache_key_differentiation_by_args() {
        let args_a = json!({"path": "/src"});
        let args_b = json!({"path": "/tests"});

        let key1 = L1Cache::cache_key("tldr_tree", &args_a);
        let key2 = L1Cache::cache_key("tldr_tree", &args_b);

        assert_ne!(key1, key2, "different args must produce different cache keys");
    }

    // -----------------------------------------------------------------------
    // (k) Benchmark: raw cache hit latency (target: <1us for HashMap lookup)
    // -----------------------------------------------------------------------
    #[test]
    fn bench_cache_hit_latency() {
        let mut cache = L1Cache::new(Duration::from_secs(60), 200);
        let result = make_result("cached structure output with realistic payload size for benchmarking");
        let key = L1Cache::cache_key(
            "tldr_structure",
            &json!({"path": "/test/file.rs", "language": "rust"}),
        );

        // Warm: insert the entry once
        cache.insert(key.clone(), result);

        // Measure: 10,000 cache hits
        let iterations = 10_000u32;
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            let _ = std::hint::black_box(cache.get(&key));
        }
        let elapsed = start.elapsed();
        let per_hit = elapsed / iterations;

        eprintln!(
            "Raw cache hit latency: {:?} per lookup ({} iterations in {:?})",
            per_hit, iterations, elapsed
        );

        // Assert: must be under 1us for raw HashMap lookup.
        // Release mode: ~128ns. Debug mode: ~400ns. Both well under 1us.
        // The 15us target includes JSON serialization overhead in the full call_tool path.
        assert!(
            per_hit < Duration::from_micros(1),
            "Cache hit too slow: {:?} (target: <1us for raw lookup)",
            per_hit
        );
    }

    // -----------------------------------------------------------------------
    // (l) Benchmark: cache key construction latency
    // -----------------------------------------------------------------------
    #[test]
    fn bench_cache_key_construction() {
        let args = json!({
            "path": "/Users/cosimo/projects/my-app/src/lib.rs",
            "language": "rust",
            "max_results": 50
        });

        // Measure: 10,000 cache key constructions
        let iterations = 10_000u32;
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            let _ = std::hint::black_box(L1Cache::cache_key("tldr_structure", &args));
        }
        let elapsed = start.elapsed();
        let per_key = elapsed / iterations;

        eprintln!(
            "Cache key construction: {:?} per key ({} iterations in {:?})",
            per_key, iterations, elapsed
        );

        // Cache key construction involves JSON key sorting + serialization.
        // Release mode: ~1.5us. Debug mode: ~6us due to unoptimized serde.
        // The important contract is the full call_tool path <15us in release.
        // We allow 10us here to avoid flaky failures in debug test builds.
        assert!(
            per_key < Duration::from_micros(10),
            "Cache key construction too slow: {:?} (target: <10us, release ~1.5us)",
            per_key
        );
    }
}
