//! Tests for Hash Index and Clone Collision Verification (Phase 6)
//!
//! Tests for:
//! - HashIndex basic operations
//! - Collision handling
//! - Clone verification to filter false positives
//!
//! Risk mitigations: S8-P1-T2, S8-P1-T6

use tldr_core::analysis::clones::{HashEntry, HashIndex, NormalizedToken, TokenCategory};

// =============================================================================
// HashIndex Basic Tests
// =============================================================================

mod hash_index_basic {
    use super::*;

    #[test]
    fn test_hash_index_new_is_empty() {
        // GIVEN: A new hash index
        let index = HashIndex::new();

        // THEN: It should be empty
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert_eq!(index.total_entries(), 0);
    }

    #[test]
    fn test_hash_index_insert_and_find() {
        // GIVEN: A hash index
        let mut index = HashIndex::new();

        // WHEN: We insert an entry
        let entry = HashEntry::new(12345, 0, 10, 20);
        index.insert(entry);

        // THEN: We should be able to find it
        let found = index.find(12345);
        assert!(found.is_some());
        let entries = found.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_idx, 0);
        assert_eq!(entries[0].start_pos, 10);
        assert_eq!(entries[0].end_pos, 20);
    }

    #[test]
    fn test_hash_index_insert_location_convenience() {
        // GIVEN: A hash index
        let mut index = HashIndex::new();

        // WHEN: We use the convenience method
        index.insert_location(99999, 1, 5, 15);

        // THEN: It should work the same as insert
        let found = index.find(99999);
        assert!(found.is_some());
        assert_eq!(found.unwrap()[0].file_idx, 1);
    }

    #[test]
    fn test_hash_index_find_nonexistent_returns_none() {
        // GIVEN: A hash index with some entries
        let mut index = HashIndex::new();
        index.insert_location(12345, 0, 0, 10);

        // WHEN: We search for a hash that doesn't exist
        let found = index.find(99999);

        // THEN: It should return None
        assert!(found.is_none());
    }

    #[test]
    fn test_hash_index_empty_queries_return_none() {
        // GIVEN: An empty hash index
        let index = HashIndex::new();

        // WHEN: We query any hash
        let found = index.find(12345);

        // THEN: It should return None
        assert!(found.is_none());
    }

    #[test]
    fn test_hash_index_len_counts_unique_hashes() {
        // GIVEN: A hash index
        let mut index = HashIndex::new();

        // WHEN: We insert multiple entries with some duplicate hashes
        index.insert_location(111, 0, 0, 10);
        index.insert_location(222, 1, 0, 10);
        index.insert_location(111, 2, 0, 10); // Same hash as first

        // THEN: len() should count unique hashes
        assert_eq!(index.len(), 2); // 111 and 222
        assert_eq!(index.total_entries(), 3); // All entries
    }
}

// =============================================================================
// HashIndex Collision Tests (S8-P1-T6)
// =============================================================================

mod hash_index_collisions {
    use super::*;

    #[test]
    fn test_hash_index_multiple_entries_same_hash() {
        // GIVEN: A hash index
        let mut index = HashIndex::new();

        // WHEN: We insert multiple entries with the same hash
        index.insert_location(12345, 0, 0, 10);
        index.insert_location(12345, 1, 5, 15);
        index.insert_location(12345, 2, 20, 30);

        // THEN: All entries should be stored under that hash
        let found = index.find(12345);
        assert!(found.is_some());
        let entries = found.unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_hash_index_find_candidates_single_entry_no_pairs() {
        // GIVEN: A hash index with single entries per hash
        let mut index = HashIndex::new();
        index.insert_location(111, 0, 0, 10);
        index.insert_location(222, 1, 0, 10);
        index.insert_location(333, 2, 0, 10);

        // WHEN: We find candidates
        let candidates = index.find_candidates();

        // THEN: No pairs should be returned (need >= 2 entries per hash)
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_hash_index_find_candidates_collision_creates_pair() {
        // GIVEN: A hash index with a collision (2 entries same hash)
        let mut index = HashIndex::new();
        index.insert_location(12345, 0, 0, 10);
        index.insert_location(12345, 1, 5, 15);

        // WHEN: We find candidates
        let candidates = index.find_candidates();

        // THEN: One pair should be returned
        assert_eq!(candidates.len(), 1);
        let (e1, e2) = candidates[0];
        assert_eq!(e1.hash, e2.hash);
    }

    #[test]
    fn test_hash_index_find_candidates_three_entries_three_pairs() {
        // GIVEN: A hash index with 3 entries same hash
        let mut index = HashIndex::new();
        index.insert_location(12345, 0, 0, 10);
        index.insert_location(12345, 1, 5, 15);
        index.insert_location(12345, 2, 20, 30);

        // WHEN: We find candidates
        let candidates = index.find_candidates();

        // THEN: 3 pairs should be returned (C(3,2) = 3)
        assert_eq!(candidates.len(), 3);
    }

    #[test]
    fn test_hash_index_multiple_buckets_with_collisions() {
        // GIVEN: A hash index with collisions in multiple buckets
        let mut index = HashIndex::new();
        // Bucket 1: hash 111, 2 entries
        index.insert_location(111, 0, 0, 10);
        index.insert_location(111, 1, 0, 10);
        // Bucket 2: hash 222, 3 entries
        index.insert_location(222, 2, 0, 10);
        index.insert_location(222, 3, 0, 10);
        index.insert_location(222, 4, 0, 10);
        // Bucket 3: hash 333, 1 entry (no collision)
        index.insert_location(333, 5, 0, 10);

        // WHEN: We find candidates
        let candidates = index.find_candidates();

        // THEN: 1 + 3 = 4 pairs (from buckets 1 and 2)
        assert_eq!(candidates.len(), 4);
    }
}

// =============================================================================
// Clone Verification Tests (S8-P1-T2)
// =============================================================================

mod clone_verification {
    use super::*;
    use tldr_core::analysis::clones::{compute_dice_similarity, verify_clone_match};

    fn make_token(value: &str, category: TokenCategory) -> NormalizedToken {
        NormalizedToken {
            value: value.to_string(),
            original: value.to_string(),
            category,
        }
    }

    fn make_tokens(values: &[&str]) -> Vec<NormalizedToken> {
        values
            .iter()
            .map(|v| make_token(v, TokenCategory::Other))
            .collect()
    }

    #[test]
    fn test_verify_clone_match_identical_tokens() {
        // GIVEN: Two identical token sequences
        let tokens1 = make_tokens(&["def", "$ID", "(", ")", ":"]);
        let tokens2 = make_tokens(&["def", "$ID", "(", ")", ":"]);

        // WHEN: We verify clone match
        let result = verify_clone_match(&tokens1, &tokens2, 0.7);

        // THEN: It should return similarity 1.0
        assert!(result.is_some());
        let similarity = result.unwrap();
        assert!((similarity - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_verify_clone_match_completely_different() {
        // GIVEN: Two completely different token sequences
        let tokens1 = make_tokens(&["def", "foo", "(", ")", ":"]);
        let tokens2 = make_tokens(&["class", "Bar", "{", "}"]);

        // WHEN: We verify clone match with threshold 0.7
        let result = verify_clone_match(&tokens1, &tokens2, 0.7);

        // THEN: It should return None (below threshold)
        assert!(result.is_none());
    }

    #[test]
    fn test_verify_clone_match_above_threshold() {
        // GIVEN: Two similar token sequences (80% similar)
        let tokens1 = make_tokens(&["def", "$ID", "(", "$ID", ")", ":", "return", "$ID"]);
        let tokens2 = make_tokens(&["def", "$ID", "(", "$ID", ")", ":", "return", "$NUM"]);

        // WHEN: We verify with threshold 0.7
        let result = verify_clone_match(&tokens1, &tokens2, 0.7);

        // THEN: It should return Some with similarity >= 0.7
        assert!(result.is_some());
        assert!(result.unwrap() >= 0.7);
    }

    #[test]
    fn test_verify_clone_match_below_threshold() {
        // GIVEN: Two sequences with ~50% similarity
        let tokens1 = make_tokens(&["a", "b", "c", "d"]);
        let tokens2 = make_tokens(&["a", "b", "x", "y"]);

        // WHEN: We verify with threshold 0.7
        let result = verify_clone_match(&tokens1, &tokens2, 0.7);

        // THEN: It should return None (below threshold)
        assert!(result.is_none());
    }

    #[test]
    fn test_hash_collision_different_content_not_clone() {
        // GIVEN: Two sequences that might have same hash but different content
        // (This tests the risk S8-P1-T2: hash collision verification)
        let tokens1 = make_tokens(&["alpha", "beta", "gamma"]);
        let tokens2 = make_tokens(&["delta", "epsilon", "zeta"]);

        // WHEN: We verify (simulating what happens after hash match)
        let result = verify_clone_match(&tokens1, &tokens2, 0.7);

        // THEN: Should return None - not a real clone
        assert!(result.is_none());
    }

    #[test]
    fn test_compute_dice_similarity_identical() {
        // GIVEN: Identical token sequences
        let tokens1 = make_tokens(&["a", "b", "c"]);
        let tokens2 = make_tokens(&["a", "b", "c"]);

        // WHEN: We compute Dice similarity
        let similarity = compute_dice_similarity(&tokens1, &tokens2);

        // THEN: Should be 1.0
        assert!((similarity - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_compute_dice_similarity_no_overlap() {
        // GIVEN: Completely different sequences
        let tokens1 = make_tokens(&["a", "b", "c"]);
        let tokens2 = make_tokens(&["x", "y", "z"]);

        // WHEN: We compute Dice similarity
        let similarity = compute_dice_similarity(&tokens1, &tokens2);

        // THEN: Should be 0.0
        assert!(similarity.abs() < 0.001);
    }

    #[test]
    fn test_compute_dice_similarity_half_overlap() {
        // GIVEN: 50% overlapping sequences
        let tokens1 = make_tokens(&["a", "b"]);
        let tokens2 = make_tokens(&["a", "c"]);

        // WHEN: We compute Dice similarity
        // Intersection = 1 (just "a")
        // Dice = 2 * 1 / (2 + 2) = 0.5
        let similarity = compute_dice_similarity(&tokens1, &tokens2);

        // THEN: Should be 0.5
        assert!((similarity - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_compute_dice_similarity_with_duplicates() {
        // GIVEN: Sequences with duplicate tokens
        let tokens1 = make_tokens(&["a", "a", "b"]);
        let tokens2 = make_tokens(&["a", "b", "b"]);

        // WHEN: We compute Dice similarity
        // Bag intersection: min(2,1) for 'a' = 1, min(1,2) for 'b' = 1 => 2
        // Dice = 2 * 2 / (3 + 3) = 4/6 = 0.666...
        let similarity = compute_dice_similarity(&tokens1, &tokens2);

        // THEN: Should be approximately 0.666
        assert!((similarity - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_compute_dice_similarity_empty_first() {
        // GIVEN: Empty first sequence
        let tokens1: Vec<NormalizedToken> = vec![];
        let tokens2 = make_tokens(&["a", "b"]);

        // WHEN: We compute Dice similarity
        let similarity = compute_dice_similarity(&tokens1, &tokens2);

        // THEN: Should be 0.0
        assert!(similarity.abs() < 0.001);
    }

    #[test]
    fn test_compute_dice_similarity_empty_second() {
        // GIVEN: Empty second sequence
        let tokens1 = make_tokens(&["a", "b"]);
        let tokens2: Vec<NormalizedToken> = vec![];

        // WHEN: We compute Dice similarity
        let similarity = compute_dice_similarity(&tokens1, &tokens2);

        // THEN: Should be 0.0
        assert!(similarity.abs() < 0.001);
    }

    #[test]
    fn test_compute_dice_similarity_both_empty() {
        // GIVEN: Both sequences empty
        let tokens1: Vec<NormalizedToken> = vec![];
        let tokens2: Vec<NormalizedToken> = vec![];

        // WHEN: We compute Dice similarity
        let similarity = compute_dice_similarity(&tokens1, &tokens2);

        // THEN: Should be 1.0 (both empty = identical)
        assert!((similarity - 1.0).abs() < 0.001);
    }
}

// =============================================================================
// Integration: HashIndex with Verification
// =============================================================================

mod hash_index_with_verification {
    use super::*;
    use std::path::PathBuf;
    use tldr_core::analysis::clones::{find_verified_clones, TokenSequence};

    fn make_token(value: &str) -> NormalizedToken {
        NormalizedToken {
            value: value.to_string(),
            original: value.to_string(),
            category: TokenCategory::Other,
        }
    }

    #[test]
    fn test_find_verified_clones_filters_hash_collisions() {
        // GIVEN: An index with a hash collision (same hash, different content)
        let mut index = HashIndex::new();

        // Two sequences with same hash but different content
        let seq1 = TokenSequence::new(
            PathBuf::from("file1.py"),
            1,
            10,
            vec![make_token("alpha"), make_token("beta"), make_token("gamma")],
            12345, // Same hash
        );
        let seq2 = TokenSequence::new(
            PathBuf::from("file2.py"),
            1,
            10,
            vec![
                make_token("delta"),
                make_token("epsilon"),
                make_token("zeta"),
            ],
            12345, // Same hash (collision!)
        );

        // Put them in a file sequences list
        let file_sequences = vec![vec![seq1], vec![seq2]];

        // Insert into index
        index.insert_location(12345, 0, 0, 0); // file 0, sequence 0
        index.insert_location(12345, 1, 0, 0); // file 1, sequence 0

        // WHEN: We find verified clones
        let verified = find_verified_clones(&index, &file_sequences, 0.7);

        // THEN: No clones should be found (they're different content)
        assert!(verified.is_empty(), "Hash collision should be filtered out");
    }

    #[test]
    fn test_find_verified_clones_keeps_real_clones() {
        // GIVEN: An index with a real clone (same hash, same/similar content)
        let mut index = HashIndex::new();

        let seq1 = TokenSequence::new(
            PathBuf::from("file1.py"),
            1,
            10,
            vec![
                make_token("def"),
                make_token("$ID"),
                make_token("("),
                make_token(")"),
            ],
            12345,
        );
        let seq2 = TokenSequence::new(
            PathBuf::from("file2.py"),
            1,
            10,
            vec![
                make_token("def"),
                make_token("$ID"),
                make_token("("),
                make_token(")"),
            ],
            12345,
        );

        let file_sequences = vec![vec![seq1], vec![seq2]];

        index.insert_location(12345, 0, 0, 0);
        index.insert_location(12345, 1, 0, 0);

        // WHEN: We find verified clones
        let verified = find_verified_clones(&index, &file_sequences, 0.7);

        // THEN: One clone pair should be found
        assert_eq!(verified.len(), 1, "Real clone should be kept");
        // Tuple is (file1_idx, seq1_idx, file2_idx, seq2_idx, similarity)
        assert!(
            (verified[0].4 - 1.0).abs() < 0.001,
            "Should have similarity 1.0"
        );
    }
}
