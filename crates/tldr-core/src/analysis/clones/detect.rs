//! Detection engine for clone detection v2.
//!
//! Implements hash-bucket matching for Type-1/Type-2 clones
//! and inverted index for Type-3 clones.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::{
    classify_clone_type, compute_dice_similarity, CloneFragment, ClonePair, CloneType,
    ClonesOptions,
};

use super::extract::FragmentData;

/// Type for deduplicating found pairs: (file1, start1, end1, file2, start2, end2)
pub type PairKey = (PathBuf, usize, usize, PathBuf, usize, usize);

/// Detect Type-1 and Type-2 clones via hash-bucket matching.
///
/// Algorithm:
/// 1. Build HashMap<u64, Vec<usize>> mapping raw_hash -> fragment indices
/// 2. For each bucket with 2+ fragments: Type-1 candidates -> verify with raw Dice
/// 3. Build HashMap<u64, Vec<usize>> mapping normalized_hash -> fragment indices
/// 4. For each bucket with 2+ fragments NOT already found: Type-2 candidates -> verify
pub fn detect_type1_type2(
    fragments: &[FragmentData],
    options: &ClonesOptions,
    found_pairs: &mut HashSet<PairKey>,
) -> Vec<ClonePair> {
    let mut clone_pairs: Vec<ClonePair> = Vec::new();

    // Step 1: Build raw hash index for Type-1 detection
    let mut raw_hash_index: HashMap<u64, Vec<usize>> = HashMap::new();
    for (idx, frag) in fragments.iter().enumerate() {
        raw_hash_index.entry(frag.raw_hash).or_default().push(idx);
    }

    // Step 2: Find Type-1 clones (exact raw token match)
    for indices in raw_hash_index.values() {
        if indices.len() < 2 {
            continue;
        }
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                if clone_pairs.len() >= options.max_clones {
                    break;
                }

                let idx_a = indices[i];
                let idx_b = indices[j];
                let frag_a = &fragments[idx_a];
                let frag_b = &fragments[idx_b];

                // Same-file exclusion (BUG-3 fix)
                if should_skip_pair(frag_a, frag_b, options) {
                    continue;
                }

                // Verify with raw Dice similarity
                let similarity = compute_dice_similarity(&frag_a.raw_tokens, &frag_b.raw_tokens);
                if similarity < 0.99 {
                    continue; // Not a Type-1 match
                }

                // Dedup check
                let pair_key = create_pair_key(frag_a, frag_b);
                if found_pairs.contains(&pair_key) {
                    continue;
                }
                found_pairs.insert(pair_key);

                // Apply type filter
                let clone_type = CloneType::Type1;
                if let Some(filter) = options.type_filter {
                    if clone_type != filter {
                        continue;
                    }
                }

                let pair = make_clone_pair(0, clone_type, similarity, frag_a, frag_b);
                clone_pairs.push(pair);
            }
            if clone_pairs.len() >= options.max_clones {
                break;
            }
        }
        if clone_pairs.len() >= options.max_clones {
            break;
        }
    }

    // Step 3: Build normalized hash index for Type-2 detection
    let mut norm_hash_index: HashMap<u64, Vec<usize>> = HashMap::new();
    for (idx, frag) in fragments.iter().enumerate() {
        norm_hash_index
            .entry(frag.normalized_hash)
            .or_default()
            .push(idx);
    }

    // Step 4: Find Type-2 clones (normalized token match, not already found as Type-1)
    for indices in norm_hash_index.values() {
        if indices.len() < 2 {
            continue;
        }
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                if clone_pairs.len() >= options.max_clones {
                    break;
                }

                let idx_a = indices[i];
                let idx_b = indices[j];
                let frag_a = &fragments[idx_a];
                let frag_b = &fragments[idx_b];

                // Same-file exclusion
                if should_skip_pair(frag_a, frag_b, options) {
                    continue;
                }

                // Dedup: skip if already found as Type-1
                let pair_key = create_pair_key(frag_a, frag_b);
                if found_pairs.contains(&pair_key) {
                    continue;
                }

                // Verify with raw Dice similarity (REQ-7: use raw for comparison)
                let raw_similarity =
                    compute_dice_similarity(&frag_a.raw_tokens, &frag_b.raw_tokens);

                // If raw sim >= 0.99, it's Type-1 (would have been caught above unless hash collision)
                let (clone_type, similarity) = if raw_similarity >= 0.99 {
                    (CloneType::Type1, raw_similarity)
                } else if raw_similarity >= 0.9 {
                    // Raw tokens are similar enough for Type-2
                    (CloneType::Type2, raw_similarity)
                } else {
                    // Check normalized similarity for Type-2
                    let norm_sim = compute_dice_similarity(
                        &frag_a.normalized_tokens,
                        &frag_b.normalized_tokens,
                    );
                    if norm_sim >= 0.9 {
                        // REQ-7: report raw similarity but classify based on normalized
                        (CloneType::Type2, raw_similarity.max(norm_sim))
                    } else {
                        continue; // Not similar enough
                    }
                };

                // Apply type filter
                if let Some(filter) = options.type_filter {
                    if clone_type != filter {
                        continue;
                    }
                }

                found_pairs.insert(pair_key);
                let pair = make_clone_pair(0, clone_type, similarity, frag_a, frag_b);
                clone_pairs.push(pair);
            }
            if clone_pairs.len() >= options.max_clones {
                break;
            }
        }
        if clone_pairs.len() >= options.max_clones {
            break;
        }
    }

    clone_pairs
}

/// Detect Type-3 clones using inverted index on raw token values.
///
/// Builds an inverted index mapping token values to fragment indices,
/// then finds candidate pairs that share enough tokens to meet threshold.
pub fn detect_type3(
    fragments: &[FragmentData],
    options: &ClonesOptions,
    found_pairs: &mut HashSet<PairKey>,
) -> Vec<ClonePair> {
    let mut clone_pairs: Vec<ClonePair> = Vec::new();

    // Build inverted index: raw token value -> [fragment_ids]
    let mut inverted: HashMap<String, Vec<usize>> = HashMap::new();
    for (frag_idx, frag) in fragments.iter().enumerate() {
        // Use unique raw token values (not normalized -- RISK-1 fix)
        let unique_tokens: HashSet<&str> =
            frag.raw_tokens.iter().map(|t| t.value.as_str()).collect();
        for token in unique_tokens {
            let entry = inverted.entry(token.to_string()).or_default();
            // Posting list cap at 500 entries (RISK-2)
            if entry.len() < 500 {
                entry.push(frag_idx);
            }
        }
    }

    // Find candidate pairs
    for (frag_idx, frag) in fragments.iter().enumerate() {
        if clone_pairs.len() >= options.max_clones {
            break;
        }

        let mut shared_counts: HashMap<usize, usize> = HashMap::new();
        let unique_tokens: HashSet<&str> =
            frag.raw_tokens.iter().map(|t| t.value.as_str()).collect();

        for token in &unique_tokens {
            if let Some(other_frags) = inverted.get(*token) {
                for &other_idx in other_frags {
                    if other_idx > frag_idx {
                        *shared_counts.entry(other_idx).or_insert(0) += 1;
                    }
                }
            }
        }

        // Filter by minimum shared tokens
        let size1 = unique_tokens.len();
        for (other_idx, shared) in shared_counts {
            if clone_pairs.len() >= options.max_clones {
                break;
            }

            let other_frag = &fragments[other_idx];
            let other_unique: HashSet<&str> = other_frag
                .raw_tokens
                .iter()
                .map(|t| t.value.as_str())
                .collect();
            let size2 = other_unique.len();

            // Quick pre-check: minimum shared tokens for threshold
            let min_shared = ((options.threshold * (size1 + size2) as f64) / 2.0).ceil() as usize;
            if shared < min_shared {
                continue;
            }

            let frag_a = frag;
            let frag_b = other_frag;

            // Same-file exclusion
            if should_skip_pair(frag_a, frag_b, options) {
                continue;
            }

            // Dedup: skip already found pairs
            let pair_key = create_pair_key(frag_a, frag_b);
            if found_pairs.contains(&pair_key) {
                continue;
            }

            // Compute raw Dice similarity
            let similarity = compute_dice_similarity(&frag_a.raw_tokens, &frag_b.raw_tokens);

            // Must meet threshold
            if similarity < options.threshold {
                continue;
            }

            // RISK-15 fix: Do NOT skip pairs with similarity >= 0.9
            // (They should have been caught by Type-1/2, but if not, still report)

            let clone_type = classify_clone_type(similarity);

            // Apply type filter
            if let Some(filter) = options.type_filter {
                if clone_type != filter {
                    continue;
                }
            }

            found_pairs.insert(pair_key);
            let pair = make_clone_pair(0, clone_type, similarity, frag_a, frag_b);
            clone_pairs.push(pair);
        }
    }

    clone_pairs
}

/// Check if a pair should be skipped based on same-file exclusion rules.
///
/// BUG-3 fix: When include_within_file=false, skip ALL same-file pairs unconditionally.
/// Always skip same-file overlapping pairs regardless of the flag.
fn should_skip_pair(frag_a: &FragmentData, frag_b: &FragmentData, options: &ClonesOptions) -> bool {
    let same_file = frag_a.file_idx == frag_b.file_idx;

    if same_file {
        // REQ-5, BUG-3: Unconditional skip when include_within_file = false
        if !options.include_within_file {
            return true;
        }

        // Always skip overlapping same-file pairs
        if ranges_overlap(
            frag_a.start_line,
            frag_a.end_line,
            frag_b.start_line,
            frag_b.end_line,
        ) {
            return true;
        }
    }

    false
}

/// Check if two line ranges overlap.
fn ranges_overlap(start1: usize, end1: usize, start2: usize, end2: usize) -> bool {
    start1 <= end2 && start2 <= end1
}

/// Create a canonical pair key for deduplication.
fn create_pair_key(frag_a: &FragmentData, frag_b: &FragmentData) -> PairKey {
    if frag_a.file < frag_b.file
        || (frag_a.file == frag_b.file && frag_a.start_line < frag_b.start_line)
    {
        (
            frag_a.file.clone(),
            frag_a.start_line,
            frag_a.end_line,
            frag_b.file.clone(),
            frag_b.start_line,
            frag_b.end_line,
        )
    } else {
        (
            frag_b.file.clone(),
            frag_b.start_line,
            frag_b.end_line,
            frag_a.file.clone(),
            frag_a.start_line,
            frag_a.end_line,
        )
    }
}

/// Build a ClonePair from two FragmentData.
fn make_clone_pair(
    id: usize,
    clone_type: CloneType,
    similarity: f64,
    frag_a: &FragmentData,
    frag_b: &FragmentData,
) -> ClonePair {
    let mut fragment1 = CloneFragment::new(
        frag_a.file.clone(),
        frag_a.start_line,
        frag_a.end_line,
        frag_a.raw_tokens.len(),
    )
    .with_preview(frag_a.preview.clone());
    if let Some(ref name) = frag_a.function_name {
        fragment1 = fragment1.with_function(name.clone());
    }

    let mut fragment2 = CloneFragment::new(
        frag_b.file.clone(),
        frag_b.start_line,
        frag_b.end_line,
        frag_b.raw_tokens.len(),
    )
    .with_preview(frag_b.preview.clone());
    if let Some(ref name) = frag_b.function_name {
        fragment2 = fragment2.with_function(name.clone());
    }

    ClonePair::new(id, clone_type, similarity, fragment1, fragment2).canonical()
}
