//! Clone detection module -- tree-sitter fragment extraction,
//! two-phase hash detection, and correct line numbers.

// All type definitions and utility functions
mod types;
pub use types::*;

// Submodules
pub mod detect;
pub mod extract;
pub mod filter;
pub mod tokenize;

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

// Re-export is_test_file from filter
pub use filter::is_test_file;

use detect::PairKey;
use extract::extract_fragments_from_file;
use filter::discover_source_files;
use tokenize::tokenize_file_v2;

/// Detect code clones in a directory using the v2 pipeline.
///
/// Pipeline:
/// 1. Discover source files
/// 2. Tokenize each file
/// 3. Extract fragments per file (tree-sitter function boundaries)
/// 4. Detect Type-1/Type-2 clones (hash buckets)
/// 5. Detect Type-3 clones (inverted index)
/// 6. Assemble ClonesReport
pub fn detect_clones(path: &Path, options: &ClonesOptions) -> anyhow::Result<ClonesReport> {
    let start = Instant::now();

    // Step 1: Discover files
    let files = discover_source_files(
        path,
        options.language.as_deref(),
        options.max_files,
        options.exclude_generated,
        options.exclude_tests,
    );

    if files.is_empty() {
        return Ok(empty_report(path, options, &start));
    }

    // Step 2: Tokenize files
    let file_tokens: Vec<tokenize::FileTokens> = files
        .iter()
        .filter_map(|f| tokenize_file_v2(f).ok())
        .collect();

    if file_tokens.is_empty() {
        return Ok(empty_report(path, options, &start));
    }

    // Step 3: Extract fragments
    let mut all_fragments: Vec<extract::FragmentData> = Vec::new();
    for (idx, ft) in file_tokens.iter().enumerate() {
        let frags = extract_fragments_from_file(
            ft,
            idx,
            options.min_tokens,
            options.min_lines,
            options.normalization,
        );
        all_fragments.extend(frags);
    }

    let total_tokens: usize = file_tokens.iter().map(|ft| ft.raw_tokens.len()).sum();

    if all_fragments.is_empty() {
        return Ok(ClonesReport {
            root: path.to_path_buf(),
            language: options
                .language
                .clone()
                .unwrap_or_else(|| "auto".to_string()),
            clone_pairs: vec![],
            clone_classes: vec![],
            stats: CloneStats {
                files_analyzed: file_tokens.len(),
                total_tokens,
                clones_found: 0,
                type1_count: 0,
                type2_count: 0,
                type3_count: 0,
                class_count: None,
                detection_time_ms: start.elapsed().as_millis() as u64,
            },
            config: CloneConfig {
                min_tokens: options.min_tokens,
                min_lines: options.min_lines,
                similarity_threshold: options.threshold,
                normalization: options.normalization,
                type_filter: options.type_filter,
            },
        });
    }

    // Step 4: Type-1 and Type-2 detection
    let mut found_pairs: HashSet<PairKey> = HashSet::new();
    let mut clone_pairs = detect::detect_type1_type2(&all_fragments, options, &mut found_pairs);

    // Step 5: Type-3 detection (if type_filter allows and not at max_clones)
    let should_detect_type3 = clone_pairs.len() < options.max_clones
        && options.type_filter.is_none_or(|t| t == CloneType::Type3);

    if should_detect_type3 {
        let type3_pairs = detect::detect_type3(&all_fragments, options, &mut found_pairs);
        clone_pairs.extend(type3_pairs);
    }

    // Assign sequential 1-indexed IDs
    for (i, pair) in clone_pairs.iter_mut().enumerate() {
        pair.id = i + 1;
    }

    // Compute statistics
    let type1_count = clone_pairs
        .iter()
        .filter(|p| p.clone_type == CloneType::Type1)
        .count();
    let type2_count = clone_pairs
        .iter()
        .filter(|p| p.clone_type == CloneType::Type2)
        .count();
    let type3_count = clone_pairs
        .iter()
        .filter(|p| p.clone_type == CloneType::Type3)
        .count();

    let (clone_classes, class_count) = if options.show_classes {
        let classes = compute_clone_classes_v2(&clone_pairs);
        let count = classes.len();
        (classes, Some(count))
    } else {
        (vec![], None)
    };

    Ok(ClonesReport {
        root: path.to_path_buf(),
        language: options
            .language
            .clone()
            .unwrap_or_else(|| "auto".to_string()),
        clone_pairs,
        clone_classes,
        stats: CloneStats {
            files_analyzed: file_tokens.len(),
            total_tokens,
            clones_found: type1_count + type2_count + type3_count,
            type1_count,
            type2_count,
            type3_count,
            class_count,
            detection_time_ms: start.elapsed().as_millis() as u64,
        },
        config: CloneConfig {
            min_tokens: options.min_tokens,
            min_lines: options.min_lines,
            similarity_threshold: options.threshold,
            normalization: options.normalization,
            type_filter: options.type_filter,
        },
    })
}

/// Compute clone classes from pairs using Union-Find (for show_classes=true).
fn compute_clone_classes_v2(pairs: &[ClonePair]) -> Vec<CloneClass> {
    use std::collections::HashMap;

    if pairs.is_empty() {
        return vec![];
    }

    // Map fragments to indices
    let mut fragment_map: HashMap<CloneFragment, usize> = HashMap::new();
    let mut fragments: Vec<CloneFragment> = Vec::new();

    for pair in pairs {
        if !fragment_map.contains_key(&pair.fragment1) {
            fragment_map.insert(pair.fragment1.clone(), fragments.len());
            fragments.push(pair.fragment1.clone());
        }
        if !fragment_map.contains_key(&pair.fragment2) {
            fragment_map.insert(pair.fragment2.clone(), fragments.len());
            fragments.push(pair.fragment2.clone());
        }
    }

    // Build Union-Find
    let mut uf = UnionFind::new(fragments.len());
    let mut pair_similarities: HashMap<(usize, usize), (f64, CloneType)> = HashMap::new();

    for pair in pairs {
        let idx1 = fragment_map[&pair.fragment1];
        let idx2 = fragment_map[&pair.fragment2];
        uf.union(idx1, idx2);
        pair_similarities.insert(
            (idx1.min(idx2), idx1.max(idx2)),
            (pair.similarity, pair.clone_type),
        );
    }

    // Extract components
    let components = uf.components();
    let mut classes: Vec<CloneClass> = Vec::new();
    let mut class_id = 1;

    for (_root, member_indices) in components {
        if member_indices.len() < 2 {
            continue;
        }

        let class_fragments: Vec<CloneFragment> = member_indices
            .iter()
            .map(|&i| fragments[i].clone())
            .collect();

        let mut total_sim = 0.0f64;
        let mut count = 0usize;
        let mut type_counts: HashMap<CloneType, usize> = HashMap::new();

        for i in 0..member_indices.len() {
            for j in (i + 1)..member_indices.len() {
                let key = (
                    member_indices[i].min(member_indices[j]),
                    member_indices[i].max(member_indices[j]),
                );
                if let Some(&(sim, ct)) = pair_similarities.get(&key) {
                    total_sim += sim;
                    count += 1;
                    *type_counts.entry(ct).or_insert(0) += 1;
                }
            }
        }

        let avg_similarity = if count > 0 {
            total_sim / count as f64
        } else {
            1.0
        };
        let dominant_type = type_counts
            .into_iter()
            .max_by_key(|&(_, c)| c)
            .map(|(t, _)| t)
            .unwrap_or(CloneType::Type1);

        classes.push(CloneClass {
            id: class_id,
            clone_type: dominant_type,
            avg_similarity,
            size: class_fragments.len(),
            fragments: class_fragments,
        });
        class_id += 1;
    }

    classes
}

/// Create an empty report for edge cases (no files, etc.)
fn empty_report(path: &Path, options: &ClonesOptions, start: &Instant) -> ClonesReport {
    ClonesReport {
        root: path.to_path_buf(),
        language: options
            .language
            .clone()
            .unwrap_or_else(|| "auto".to_string()),
        clone_pairs: vec![],
        clone_classes: vec![],
        stats: CloneStats {
            files_analyzed: 0,
            total_tokens: 0,
            clones_found: 0,
            type1_count: 0,
            type2_count: 0,
            type3_count: 0,
            class_count: None,
            detection_time_ms: start.elapsed().as_millis() as u64,
        },
        config: CloneConfig {
            min_tokens: options.min_tokens,
            min_lines: options.min_lines,
            similarity_threshold: options.threshold,
            normalization: options.normalization,
            type_filter: options.type_filter,
        },
    }
}
