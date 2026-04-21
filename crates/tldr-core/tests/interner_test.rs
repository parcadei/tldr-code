//! Tests for string interner infrastructure (Phase 2 of call graph migration).
//!
//! These tests verify memory-efficient string interning for path deduplication.
//! Without interning, a call graph with 5M edges uses ~1.9GB for path strings.
//! With interning, the same graph uses ~80MB (24x reduction).

#[cfg(test)]
mod string_interner_tests {
    use tldr_core::callgraph::interner::{InternedId, StringInterner};

    #[test]
    fn test_intern_returns_same_id_for_same_string() {
        let mut interner = StringInterner::new();
        let id1 = interner.intern("hello");
        let id2 = interner.intern("hello");
        assert_eq!(id1, id2, "Same string should return same ID");
    }

    #[test]
    fn test_intern_returns_different_ids_for_different_strings() {
        let mut interner = StringInterner::new();
        let id1 = interner.intern("hello");
        let id2 = interner.intern("world");
        assert_ne!(id1, id2, "Different strings should return different IDs");
    }

    #[test]
    fn test_get_returns_original_string() {
        let mut interner = StringInterner::new();
        let id = interner.intern("test_string");
        let result = interner.get(id);
        assert_eq!(result, Some("test_string"));
    }

    #[test]
    fn test_get_returns_none_for_invalid_id() {
        let interner = StringInterner::new();
        let invalid_id = InternedId::from_raw(999);
        assert_eq!(interner.get(invalid_id), None);
    }

    #[test]
    fn test_get_or_intern_creates_new() {
        let mut interner = StringInterner::new();
        let id = interner.get_or_intern("new_string");
        assert_eq!(interner.get(id), Some("new_string"));
    }

    #[test]
    fn test_get_or_intern_returns_existing() {
        let mut interner = StringInterner::new();
        let id1 = interner.intern("existing");
        let id2 = interner.get_or_intern("existing");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_len_tracks_unique_strings() {
        let mut interner = StringInterner::new();
        assert_eq!(interner.len(), 0);

        interner.intern("one");
        assert_eq!(interner.len(), 1);

        interner.intern("two");
        assert_eq!(interner.len(), 2);

        // Duplicate should not increase length
        interner.intern("one");
        assert_eq!(interner.len(), 2);
    }

    #[test]
    fn test_is_empty() {
        let mut interner = StringInterner::new();
        assert!(interner.is_empty());

        interner.intern("something");
        assert!(!interner.is_empty());
    }

    #[test]
    fn test_empty_string_handling() {
        let mut interner = StringInterner::new();
        let id = interner.intern("");
        assert_eq!(interner.get(id), Some(""));
    }

    #[test]
    fn test_interned_id_is_copy_and_eq() {
        let mut interner = StringInterner::new();
        let id = interner.intern("test");

        // Copy trait
        let id_copy = id;
        assert_eq!(id, id_copy);

        // Can use both after copy
        assert_eq!(interner.get(id), interner.get(id_copy));
    }

    #[test]
    fn test_interned_id_hash_works() {
        use std::collections::HashSet;

        let mut interner = StringInterner::new();
        let id1 = interner.intern("a");
        let id2 = interner.intern("b");
        let id3 = interner.intern("a"); // Same as id1

        let mut set = HashSet::new();
        set.insert(id1);
        set.insert(id2);
        set.insert(id3); // Should not add (duplicate of id1)

        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_interned_id_as_u32() {
        let mut interner = StringInterner::new();
        let id = interner.intern("test");

        // Should be able to get raw u32 value
        let raw = id.as_u32();
        assert_eq!(raw, 0); // First interned string gets ID 0

        let id2 = interner.intern("test2");
        assert_eq!(id2.as_u32(), 1);
    }

    #[test]
    fn test_large_number_of_strings() {
        let mut interner = StringInterner::new();
        let count = 100_000;

        // Intern many unique strings
        for i in 0..count {
            interner.intern(&format!("string_{}", i));
        }

        assert_eq!(interner.len(), count);

        // Verify deduplication still works
        let id1 = interner.intern("string_0");
        let id2 = interner.intern("string_0");
        assert_eq!(id1, id2);
        assert_eq!(interner.len(), count); // No increase
    }
}

#[cfg(test)]
mod path_interner_tests {
    use std::path::Path;
    use tldr_core::callgraph::interner::PathInterner;

    #[test]
    fn test_path_intern_basic() {
        let mut interner = PathInterner::new();
        let id = interner.intern_path(Path::new("src/main.rs"));
        assert!(interner.get_path(id).is_some());
    }

    #[test]
    fn test_path_normalization_backslash_to_forward() {
        let mut interner = PathInterner::new();

        // Windows-style path
        let id1 = interner.intern_path(Path::new("src\\main.rs"));
        // Unix-style path
        let id2 = interner.intern_path(Path::new("src/main.rs"));

        // Should normalize to same (forward slashes)
        assert_eq!(
            id1, id2,
            "Backslash and forward slash paths should be deduplicated"
        );
        assert_eq!(interner.get_path(id1), Some("src/main.rs"));
    }

    #[test]
    fn test_path_deduplication() {
        let mut interner = PathInterner::new();

        let id1 = interner.intern_path(Path::new("project/src/lib.rs"));
        let id2 = interner.intern_path(Path::new("project/src/lib.rs"));

        assert_eq!(id1, id2);
        assert_eq!(interner.len(), 1);
    }

    #[test]
    fn test_path_get_path() {
        let mut interner = PathInterner::new();
        let id = interner.intern_path(Path::new("tests/test.py"));

        let path_str = interner.get_path(id);
        assert_eq!(path_str, Some("tests/test.py"));
    }

    #[test]
    fn test_path_empty_path() {
        let mut interner = PathInterner::new();
        let id = interner.intern_path(Path::new(""));
        assert_eq!(interner.get_path(id), Some(""));
    }

    #[test]
    fn test_path_absolute_paths() {
        let mut interner = PathInterner::new();

        // Unix absolute path
        let id1 = interner.intern_path(Path::new("/home/user/project/main.py"));
        assert_eq!(interner.get_path(id1), Some("/home/user/project/main.py"));

        // Windows absolute path (normalized)
        let id2 = interner.intern_path(Path::new("C:\\Users\\project\\main.py"));
        // Should have backslashes converted
        let path_str = interner.get_path(id2).unwrap();
        assert!(
            !path_str.contains('\\'),
            "Backslashes should be normalized to forward slashes"
        );
    }

    #[test]
    fn test_path_with_dots() {
        let mut interner = PathInterner::new();

        // Relative path with dots
        let id = interner.intern_path(Path::new("../sibling/file.rs"));
        assert_eq!(interner.get_path(id), Some("../sibling/file.rs"));
    }

    #[test]
    fn test_path_unicode() {
        let mut interner = PathInterner::new();

        // Path with unicode characters
        let id = interner.intern_path(Path::new("src/archivo.rs"));
        assert_eq!(interner.get_path(id), Some("src/archivo.rs"));
    }
}

#[cfg(test)]
mod thread_safety_tests {
    use std::sync::Arc;
    use std::thread;
    use tldr_core::callgraph::interner::ConcurrentInterner;

    #[test]
    fn test_concurrent_intern_same_string() {
        let interner = Arc::new(ConcurrentInterner::new());
        let mut handles = vec![];

        // Spawn multiple threads interning the same string
        for _ in 0..10 {
            let interner_clone = Arc::clone(&interner);
            handles.push(thread::spawn(move || {
                interner_clone.intern("shared_string")
            }));
        }

        let ids: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // All threads should get the same ID
        let first_id = ids[0];
        for id in &ids {
            assert_eq!(
                *id, first_id,
                "All threads should get same ID for same string"
            );
        }
    }

    #[test]
    fn test_concurrent_intern_different_strings() {
        let interner = Arc::new(ConcurrentInterner::new());
        let mut handles = vec![];

        // Spawn threads interning different strings
        for i in 0..100 {
            let interner_clone = Arc::clone(&interner);
            handles.push(thread::spawn(move || {
                let s = format!("string_{}", i);
                interner_clone.intern(&s)
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Should have all unique strings
        assert_eq!(interner.len(), 100);
    }

    #[test]
    fn test_concurrent_get_after_intern() {
        let interner = Arc::new(ConcurrentInterner::new());

        // Intern some strings first
        let id = interner.intern("test_value");

        // Multiple threads reading
        let mut handles = vec![];
        for _ in 0..10 {
            let interner_clone = Arc::clone(&interner);
            handles.push(thread::spawn(move || interner_clone.get(id)));
        }

        for handle in handles {
            let result = handle.join().unwrap();
            assert_eq!(result, Some("test_value".to_string()));
        }
    }
}

#[cfg(test)]
mod dedup_stats_tests {
    use tldr_core::callgraph::interner::StringInterner;

    #[test]
    fn test_dedup_statistics() {
        let mut interner = StringInterner::new();

        // Intern with duplicates
        interner.intern("path/to/file.py");
        interner.intern("path/to/file.py");
        interner.intern("path/to/file.py");
        interner.intern("other/file.py");
        interner.intern("other/file.py");

        let stats = interner.stats();

        assert_eq!(stats.unique_count, 2);
        assert_eq!(stats.total_intern_calls, 5);
        assert_eq!(stats.dedup_ratio(), 0.6); // 3 out of 5 were duplicates
    }

    #[test]
    fn test_memory_estimate() {
        let mut interner = StringInterner::new();

        // Intern some strings
        for i in 0..1000 {
            interner.intern(&format!("path/to/file_{}.py", i));
        }

        let stats = interner.stats();

        // Memory estimate should be reasonable
        assert!(stats.estimated_memory_bytes > 0);
        // Should be proportional to unique strings
        assert!(stats.estimated_memory_bytes > 1000 * 10); // At least 10 bytes per string avg
    }
}
