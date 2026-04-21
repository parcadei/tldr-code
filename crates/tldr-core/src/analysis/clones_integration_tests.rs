//! Integration tests for clones detection against the click corpus.
//!
//! These tests run clones::detect_clones on real-world Python code
//! (the click library) and verify:
//!
//! 1. No false positives: every reported clone pair contains similar code
//! 2. Accurate line numbers: start_line/end_line match real source positions
//! 3. Previews show real code from the source files
//! 4. JSON output round-trips correctly
//! 5. Regression vs v1: documented false positives from v1 are eliminated
//!
//! The click corpus is expected at /tmp/test-dead-corpora-v2/click/src/click.
//! Tests that require the corpus are marked #[ignore] so they don't break CI.

use std::path::Path;

use crate::analysis::clones::{self, CloneType, ClonesOptions, ClonesReport, NormalizationMode};

// =============================================================================
// Helper: Check if click corpus exists
// =============================================================================

fn click_corpus_path() -> Option<&'static Path> {
    let p = Path::new("/tmp/test-dead-corpora-v2/click/src/click");
    if p.exists() && p.is_dir() {
        Some(p)
    } else {
        None
    }
}

fn default_click_options() -> ClonesOptions {
    ClonesOptions {
        min_tokens: 25,
        min_lines: 5,
        threshold: 0.7,
        type_filter: None,
        normalization: NormalizationMode::All,
        language: Some("python".to_string()),
        show_classes: false,
        include_within_file: false,
        max_clones: 100,
        max_files: 1000,
        exclude_generated: false,
        exclude_tests: false,
    }
}

// =============================================================================
// Test: clones runs successfully on click corpus
// =============================================================================

/// The detector must not panic or error on a real codebase.
#[test]
#[ignore] // requires click corpus at /tmp
fn click_corpus_runs_without_error() {
    let path = click_corpus_path()
        .expect("click corpus not found at /tmp/test-dead-corpora-v2/click/src/click");
    let opts = default_click_options();
    let report = clones::detect_clones(path, &opts);
    assert!(
        report.is_ok(),
        "detect_clones should succeed on click corpus: {:?}",
        report.err()
    );
}

// =============================================================================
// Test: Line numbers are accurate
// =============================================================================

/// For every clone pair, the reported start_line/end_line must be valid:
/// 1. start_line >= 1
/// 2. end_line >= start_line
/// 3. The lines at start_line..=end_line in the file contain actual code
#[test]
#[ignore]
fn click_corpus_line_numbers_are_valid() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let report = clones::detect_clones(path, &opts).unwrap();

    for pair in &report.clone_pairs {
        for frag in [&pair.fragment1, &pair.fragment2] {
            // Basic validity
            assert!(
                frag.start_line >= 1,
                "start_line must be >= 1, got {}",
                frag.start_line
            );
            assert!(
                frag.end_line >= frag.start_line,
                "end_line ({}) must be >= start_line ({})",
                frag.end_line,
                frag.start_line
            );

            // Read the file and check the lines exist
            let source = std::fs::read_to_string(&frag.file)
                .unwrap_or_else(|e| panic!("Cannot read {:?}: {}", frag.file, e));
            let lines: Vec<&str> = source.lines().collect();
            let total_lines = lines.len();

            assert!(
                frag.start_line <= total_lines,
                "start_line {} exceeds file length {} in {:?}",
                frag.start_line,
                total_lines,
                frag.file
            );
            assert!(
                frag.end_line <= total_lines,
                "end_line {} exceeds file length {} in {:?}",
                frag.end_line,
                total_lines,
                frag.file
            );

            // The code at these lines should not be entirely blank
            let code_lines: Vec<&str> = lines[(frag.start_line - 1)..frag.end_line].to_vec();
            let non_blank = code_lines.iter().filter(|l| !l.trim().is_empty()).count();
            assert!(
                non_blank > 0,
                "Lines {}..={} in {:?} are entirely blank",
                frag.start_line,
                frag.end_line,
                frag.file
            );
        }
    }
}

/// Line numbers should NOT all be identical (which would indicate fabrication).
/// v1 had a bug where many pairs got the same (start_line, end_line) due to
/// fixed 25-token windows.
#[test]
#[ignore]
fn click_corpus_line_numbers_are_not_all_identical() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let report = clones::detect_clones(path, &opts).unwrap();

    if report.clone_pairs.len() < 2 {
        return; // Not enough pairs to check
    }

    let mut unique_ranges = std::collections::HashSet::new();
    for pair in &report.clone_pairs {
        unique_ranges.insert((
            pair.fragment1.file.clone(),
            pair.fragment1.start_line,
            pair.fragment1.end_line,
        ));
        unique_ranges.insert((
            pair.fragment2.file.clone(),
            pair.fragment2.start_line,
            pair.fragment2.end_line,
        ));
    }

    // If all pairs have the same range, that's suspicious
    assert!(
        unique_ranges.len() > 1,
        "All {} clone pairs have the same fragment ranges -- likely fabricated line numbers",
        report.clone_pairs.len()
    );
}

// =============================================================================
// Test: Previews contain real source code
// =============================================================================

/// Every fragment's preview should contain actual code from the file at the
/// reported line numbers.
#[test]
#[ignore]
fn click_corpus_previews_match_source() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let report = clones::detect_clones(path, &opts).unwrap();

    for pair in &report.clone_pairs {
        for frag in [&pair.fragment1, &pair.fragment2] {
            let preview = frag.preview.as_deref().unwrap_or("");

            // Preview must not be empty (BUG-5 fix)
            assert!(
                !preview.is_empty(),
                "Pair {}: preview is empty for {:?}:{}",
                pair.id,
                frag.file,
                frag.start_line
            );

            // The preview text should appear in the source file
            let source = std::fs::read_to_string(&frag.file).unwrap();
            let lines: Vec<&str> = source.lines().collect();
            let code_at_lines = lines[(frag.start_line - 1)..frag.end_line].join("\n");

            // The preview is truncated to ~100 chars, so check that the
            // first meaningful line of the preview appears in the source lines
            let first_preview_line = preview.lines().next().unwrap_or("").trim();
            if !first_preview_line.is_empty() && !first_preview_line.ends_with("...") {
                assert!(
                    code_at_lines.contains(first_preview_line),
                    "Pair {}: preview first line {:?} not found in source lines {}..={} of {:?}\nSource:\n{}",
                    pair.id, first_preview_line, frag.start_line, frag.end_line, frag.file,
                    &code_at_lines[..code_at_lines.len().min(200)]
                );
            }
        }
    }
}

// =============================================================================
// Test: No false positives from import blocks
// =============================================================================

/// v1 reported __init__.py import blocks as Type-1 clones (BUG-4).
/// v2 should not report import-only blocks as clones because imports
/// are stripped during tokenization.
#[test]
#[ignore]
fn click_corpus_no_init_import_false_positives() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let report = clones::detect_clones(path, &opts).unwrap();

    for pair in &report.clone_pairs {
        let f1_is_init = pair
            .fragment1
            .file
            .file_name()
            .is_some_and(|n| n == "__init__.py");
        let f2_is_init = pair
            .fragment2
            .file
            .file_name()
            .is_some_and(|n| n == "__init__.py");

        if f1_is_init && f2_is_init {
            // If both fragments are in __init__.py, verify they are NOT
            // just import blocks (which v1 incorrectly reported)
            let source = std::fs::read_to_string(&pair.fragment1.file).unwrap();
            let lines: Vec<&str> = source.lines().collect();
            let code1: Vec<&str> =
                lines[(pair.fragment1.start_line - 1)..pair.fragment1.end_line].to_vec();
            let all_imports_1 = code1.iter().all(|l| {
                let t = l.trim();
                t.is_empty()
                    || t.starts_with("from ")
                    || t.starts_with("import ")
                    || t.starts_with("#")
            });

            if all_imports_1 {
                panic!(
                    "Pair {} is a false positive: both fragments in __init__.py are import blocks (lines {}..={} and {}..={})",
                    pair.id,
                    pair.fragment1.start_line, pair.fragment1.end_line,
                    pair.fragment2.start_line, pair.fragment2.end_line,
                );
            }
        }
    }
}

// =============================================================================
// Test: Token counts are not fixed at 25
// =============================================================================

/// v1 always reported 25 tokens per fragment (BUG-6: fixed window).
/// v2 should report varying token counts based on function boundaries.
#[test]
#[ignore]
fn click_corpus_token_counts_vary() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let report = clones::detect_clones(path, &opts).unwrap();

    if report.clone_pairs.is_empty() {
        return;
    }

    let mut token_counts = std::collections::HashSet::new();
    for pair in &report.clone_pairs {
        token_counts.insert(pair.fragment1.tokens);
        token_counts.insert(pair.fragment2.tokens);
    }

    // v2 should produce varied token counts, not all 25
    if report.clone_pairs.len() >= 3 {
        assert!(
            token_counts.len() > 1,
            "All {} fragments have the same token count {:?} -- likely fixed window (BUG-6)",
            report.clone_pairs.len() * 2,
            token_counts
        );
    }
}

// =============================================================================
// Test: include_within_file=false respected (BUG-3 fix)
// =============================================================================

/// With include_within_file=false (default), no pair should have both fragments
/// in the same file.
#[test]
#[ignore]
fn click_corpus_no_within_file_pairs_by_default() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    // include_within_file defaults to false
    assert!(!opts.include_within_file);

    let report = clones::detect_clones(path, &opts).unwrap();

    for pair in &report.clone_pairs {
        assert_ne!(
            pair.fragment1.file, pair.fragment2.file,
            "Pair {} has both fragments in {:?} but include_within_file=false",
            pair.id, pair.fragment1.file
        );
    }
}

// =============================================================================
// Test: min_lines is enforced (BUG-2 fix)
// =============================================================================

/// Every reported fragment should span at least min_lines lines.
#[test]
#[ignore]
fn click_corpus_min_lines_enforced() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let min_lines = opts.min_lines;

    let report = clones::detect_clones(path, &opts).unwrap();

    for pair in &report.clone_pairs {
        for frag in [&pair.fragment1, &pair.fragment2] {
            let line_count = frag.end_line - frag.start_line + 1;
            assert!(
                line_count >= min_lines,
                "Pair {}: fragment {:?}:{}..={} spans {} lines, below min_lines={}",
                pair.id,
                frag.file,
                frag.start_line,
                frag.end_line,
                line_count,
                min_lines
            );
        }
    }
}

// =============================================================================
// Test: Similarity scores are valid
// =============================================================================

/// All similarity scores must be in [threshold, 1.0] and clone types
/// must match the score.
#[test]
#[ignore]
fn click_corpus_similarity_scores_valid() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let threshold = opts.threshold;

    let report = clones::detect_clones(path, &opts).unwrap();

    for pair in &report.clone_pairs {
        assert!(
            pair.similarity >= threshold,
            "Pair {}: similarity {} below threshold {}",
            pair.id,
            pair.similarity,
            threshold
        );
        assert!(
            pair.similarity <= 1.0,
            "Pair {}: similarity {} exceeds 1.0",
            pair.id,
            pair.similarity
        );

        // Clone type classification sanity
        match pair.clone_type {
            CloneType::Type1 => {
                assert!(
                    pair.similarity >= 0.99,
                    "Pair {}: Type-1 but similarity {} < 0.99",
                    pair.id,
                    pair.similarity
                );
            }
            CloneType::Type2 => {
                assert!(
                    pair.similarity >= 0.9,
                    "Pair {}: Type-2 but similarity {} < 0.9",
                    pair.id,
                    pair.similarity
                );
            }
            CloneType::Type3 => {
                assert!(
                    pair.similarity >= threshold && pair.similarity < 0.9,
                    "Pair {}: Type-3 but similarity {} not in [{}, 0.9)",
                    pair.id,
                    pair.similarity,
                    threshold
                );
            }
        }
    }
}

// =============================================================================
// Test: JSON round-trip
// =============================================================================

/// The ClonesReport must serialize to JSON and deserialize back identically.
#[test]
#[ignore]
fn click_corpus_json_round_trip() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let report = clones::detect_clones(path, &opts).unwrap();

    let json_str = serde_json::to_string_pretty(&report).expect("serialize to JSON");
    let report2: ClonesReport = serde_json::from_str(&json_str).expect("deserialize from JSON");

    assert_eq!(report.clone_pairs.len(), report2.clone_pairs.len());
    assert_eq!(report.stats.files_analyzed, report2.stats.files_analyzed);
    assert_eq!(report.stats.clones_found, report2.stats.clones_found);
    assert_eq!(report.stats.type1_count, report2.stats.type1_count);
    assert_eq!(report.stats.type2_count, report2.stats.type2_count);
    assert_eq!(report.stats.type3_count, report2.stats.type3_count);

    for (p1, p2) in report.clone_pairs.iter().zip(report2.clone_pairs.iter()) {
        assert_eq!(p1.id, p2.id);
        assert_eq!(p1.clone_type, p2.clone_type);
        assert!((p1.similarity - p2.similarity).abs() < 1e-10);
        assert_eq!(p1.fragment1.file, p2.fragment1.file);
        assert_eq!(p1.fragment1.start_line, p2.fragment1.start_line);
        assert_eq!(p1.fragment1.end_line, p2.fragment1.end_line);
    }
}

// =============================================================================
// Test: Manual cross-verification of clone content
// =============================================================================

/// For each clone pair, verify that the source code at the reported positions
/// is actually similar -- not just coincidentally hashed the same.
#[test]
#[ignore]
fn click_corpus_clone_content_actually_similar() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let report = clones::detect_clones(path, &opts).unwrap();

    for pair in &report.clone_pairs {
        let source1 = std::fs::read_to_string(&pair.fragment1.file).unwrap();
        let source2 = std::fs::read_to_string(&pair.fragment2.file).unwrap();

        let lines1: Vec<&str> = source1.lines().collect();
        let lines2: Vec<&str> = source2.lines().collect();

        let code1: String =
            lines1[(pair.fragment1.start_line - 1)..pair.fragment1.end_line].join("\n");
        let code2: String =
            lines2[(pair.fragment2.start_line - 1)..pair.fragment2.end_line].join("\n");

        // Both code blocks should contain at least some non-trivial code
        let non_trivial_1 = code1
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty()
                    && !t.starts_with("#")
                    && !t.starts_with("from ")
                    && !t.starts_with("import ")
            })
            .count();

        let non_trivial_2 = code2
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty()
                    && !t.starts_with("#")
                    && !t.starts_with("from ")
                    && !t.starts_with("import ")
            })
            .count();

        assert!(
            non_trivial_1 > 0,
            "Pair {}: fragment1 at {:?}:{}..={} has no non-trivial code\nCode:\n{}",
            pair.id,
            pair.fragment1.file,
            pair.fragment1.start_line,
            pair.fragment1.end_line,
            &code1[..code1.len().min(300)]
        );

        assert!(
            non_trivial_2 > 0,
            "Pair {}: fragment2 at {:?}:{}..={} has no non-trivial code\nCode:\n{}",
            pair.id,
            pair.fragment2.file,
            pair.fragment2.start_line,
            pair.fragment2.end_line,
            &code2[..code2.len().min(300)]
        );

        // Simple structural similarity check: count shared non-trivial tokens
        // between the two code blocks (whitespace-split, lowered)
        let tokens1: std::collections::HashSet<String> = code1
            .split_whitespace()
            .map(|s| s.to_lowercase())
            .filter(|s| s.len() > 1)
            .collect();
        let tokens2: std::collections::HashSet<String> = code2
            .split_whitespace()
            .map(|s| s.to_lowercase())
            .filter(|s| s.len() > 1)
            .collect();

        let shared = tokens1.intersection(&tokens2).count();
        let total = tokens1.len().max(tokens2.len());

        if total > 0 {
            let overlap = shared as f64 / total as f64;
            // For a pair reported at threshold 0.7, the source tokens should
            // share at least some overlap. We use 0.15 as a sanity floor --
            // even structurally different code with renamed vars should share
            // keywords like def, return, if, etc.
            assert!(
                overlap >= 0.15,
                "Pair {}: source code token overlap is only {:.1}% ({}/{}), likely false positive.\n\
                 Fragment1: {:?}:{}..={}\n\
                 Fragment2: {:?}:{}..={}\n\
                 Code1 (first 200):\n{}\n\
                 Code2 (first 200):\n{}",
                pair.id, overlap * 100.0, shared, total,
                pair.fragment1.file, pair.fragment1.start_line, pair.fragment1.end_line,
                pair.fragment2.file, pair.fragment2.start_line, pair.fragment2.end_line,
                &code1[..code1.len().min(200)],
                &code2[..code2.len().min(200)],
            );
        }
    }
}

// =============================================================================
// Test: Stats are consistent
// =============================================================================

/// The stats counters should match the actual clone_pairs data.
#[test]
#[ignore]
fn click_corpus_stats_consistent() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let report = clones::detect_clones(path, &opts).unwrap();

    let actual_type1 = report
        .clone_pairs
        .iter()
        .filter(|p| p.clone_type == CloneType::Type1)
        .count();
    let actual_type2 = report
        .clone_pairs
        .iter()
        .filter(|p| p.clone_type == CloneType::Type2)
        .count();
    let actual_type3 = report
        .clone_pairs
        .iter()
        .filter(|p| p.clone_type == CloneType::Type3)
        .count();

    assert_eq!(
        report.stats.type1_count, actual_type1,
        "type1_count mismatch"
    );
    assert_eq!(
        report.stats.type2_count, actual_type2,
        "type2_count mismatch"
    );
    assert_eq!(
        report.stats.type3_count, actual_type3,
        "type3_count mismatch"
    );
    assert_eq!(
        report.stats.clones_found,
        actual_type1 + actual_type2 + actual_type3,
        "clones_found mismatch"
    );
    assert_eq!(
        report.clone_pairs.len(),
        report.stats.clones_found,
        "clone_pairs.len() != stats.clones_found"
    );

    // Files analyzed should be > 0 for click corpus
    assert!(report.stats.files_analyzed > 0, "no files analyzed");
    assert!(report.stats.total_tokens > 0, "no tokens extracted");
}

// =============================================================================
// Test: IDs are sequential and 1-indexed
// =============================================================================

#[test]
#[ignore]
fn click_corpus_ids_sequential() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let report = clones::detect_clones(path, &opts).unwrap();

    for (i, pair) in report.clone_pairs.iter().enumerate() {
        assert_eq!(
            pair.id,
            i + 1,
            "Pair at index {} has id {} (expected {})",
            i,
            pair.id,
            i + 1
        );
    }
}

// =============================================================================
// Test: With include_within_file=true, we can get within-file pairs
// =============================================================================

#[test]
#[ignore]
fn click_corpus_within_file_when_enabled() {
    let path = click_corpus_path().expect("click corpus not found");
    let mut opts = default_click_options();
    opts.include_within_file = true;

    let report = clones::detect_clones(path, &opts).unwrap();

    // With within-file enabled on click (which has core.py with 3400+ lines),
    // we should find at least some within-file pairs
    let within_file_count = report
        .clone_pairs
        .iter()
        .filter(|p| p.fragment1.file == p.fragment2.file)
        .count();

    // Just verify it doesn't panic and we get some results
    // (within-file pairs are optional -- click may or may not have them)
    eprintln!(
        "Within-file pairs with include_within_file=true: {} out of {} total",
        within_file_count,
        report.clone_pairs.len()
    );
}

// =============================================================================
// Test: Exclude tests filter works
// =============================================================================

#[test]
#[ignore]
fn click_corpus_exclude_tests_filter() {
    let path = Path::new("/tmp/test-dead-corpora-v2/click");
    if !path.exists() {
        return;
    }

    let mut opts = default_click_options();
    opts.exclude_tests = true;

    let report = clones::detect_clones(path, &opts).unwrap();

    // No fragment should reference a test file
    for pair in &report.clone_pairs {
        for frag in [&pair.fragment1, &pair.fragment2] {
            let path_str = frag.file.to_string_lossy();
            assert!(
                !path_str.contains("/tests/") && !path_str.contains("test_"),
                "Fragment {:?} looks like a test file but exclude_tests=true",
                frag.file
            );
        }
    }
}

// =============================================================================
// Test: Comprehensive click corpus analysis (diagnostic, not assertion-heavy)
// =============================================================================

/// Run the full analysis and print a summary. This test always passes
/// but provides diagnostic output for manual review.
#[test]
#[ignore]
fn click_corpus_diagnostic_summary() {
    let path = click_corpus_path().expect("click corpus not found");
    let opts = default_click_options();
    let report = clones::detect_clones(path, &opts).unwrap();

    eprintln!("\n=== clones Click Corpus Analysis ===");
    eprintln!("Files analyzed: {}", report.stats.files_analyzed);
    eprintln!("Total tokens: {}", report.stats.total_tokens);
    eprintln!("Clones found: {}", report.stats.clones_found);
    eprintln!("  Type-1: {}", report.stats.type1_count);
    eprintln!("  Type-2: {}", report.stats.type2_count);
    eprintln!("  Type-3: {}", report.stats.type3_count);
    eprintln!("Detection time: {}ms", report.stats.detection_time_ms);
    eprintln!();

    for pair in &report.clone_pairs {
        let f1_name = pair
            .fragment1
            .file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let f2_name = pair
            .fragment2
            .file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let func1 = pair.fragment1.function.as_deref().unwrap_or("<none>");
        let func2 = pair.fragment2.function.as_deref().unwrap_or("<none>");
        let preview1 = pair
            .fragment1
            .preview
            .as_deref()
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("");
        let preview2 = pair
            .fragment2
            .preview
            .as_deref()
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("");

        eprintln!(
            "Pair {:2}: {:?} sim={:.3} {}:{}..={} [{}] <-> {}:{}..={} [{}]",
            pair.id,
            pair.clone_type,
            pair.similarity,
            f1_name,
            pair.fragment1.start_line,
            pair.fragment1.end_line,
            func1,
            f2_name,
            pair.fragment2.start_line,
            pair.fragment2.end_line,
            func2,
        );
        if !preview1.is_empty() {
            eprintln!("         frag1: {}", &preview1[..preview1.len().min(80)]);
        }
        if !preview2.is_empty() {
            eprintln!("         frag2: {}", &preview2[..preview2.len().min(80)]);
        }
    }
    eprintln!("=== End Analysis ===\n");
}

// =============================================================================
// Test with synthetic known-duplicate Python files
// =============================================================================
// These do NOT require the click corpus and run in CI.

/// Create temp files with known duplications and verify detection.
///
/// Each file has at least 2 substantial functions (>= 5 lines, >= 15 tokens)
/// so tree-sitter extracts proper function-level fragments (no sliding window fallback).
#[test]
fn synthetic_known_duplicates_detected() {
    let dir = tempfile::TempDir::new().unwrap();

    // File A: process_data (identical in B) + transform_output (unique, substantial)
    let file_a = "def process_data(items):\n\
                   \x20   result = []\n\
                   \x20   for item in items:\n\
                   \x20       if item is not None:\n\
                   \x20           value = item.strip()\n\
                   \x20           if len(value) > 0:\n\
                   \x20               result.append(value.lower())\n\
                   \x20   return sorted(result)\n\
                   \n\
                   \n\
                   def transform_output(data, prefix):\n\
                   \x20   output = {}\n\
                   \x20   for key, value in data.items():\n\
                   \x20       new_key = prefix + str(key)\n\
                   \x20       output[new_key] = str(value)\n\
                   \x20       if value is None:\n\
                   \x20           output[new_key] = \"missing\"\n\
                   \x20   return output\n";

    // File B: process_data (identical to A) + compute_stats (unique, substantial)
    let file_b = "def process_data(items):\n\
                   \x20   result = []\n\
                   \x20   for item in items:\n\
                   \x20       if item is not None:\n\
                   \x20           value = item.strip()\n\
                   \x20           if len(value) > 0:\n\
                   \x20               result.append(value.lower())\n\
                   \x20   return sorted(result)\n\
                   \n\
                   \n\
                   def compute_stats(numbers):\n\
                   \x20   total = sum(numbers)\n\
                   \x20   count = len(numbers)\n\
                   \x20   average = total / count\n\
                   \x20   maximum = max(numbers)\n\
                   \x20   minimum = min(numbers)\n\
                   \x20   return {\"avg\": average, \"max\": maximum, \"min\": minimum}\n";

    // File C: handle_records (renamed version of process_data) + format_report (unique)
    let file_c = "def handle_records(entries):\n\
                   \x20   output = []\n\
                   \x20   for entry in entries:\n\
                   \x20       if entry is not None:\n\
                   \x20           val = entry.strip()\n\
                   \x20           if len(val) > 0:\n\
                   \x20               output.append(val.lower())\n\
                   \x20   return sorted(output)\n\
                   \n\
                   \n\
                   def format_report(title, sections):\n\
                   \x20   lines = [title]\n\
                   \x20   for section in sections:\n\
                   \x20       header = section.get(\"name\", \"\")\n\
                   \x20       body = section.get(\"content\", \"\")\n\
                   \x20       lines.append(header)\n\
                   \x20       lines.append(body)\n\
                   \x20   return \"\\n\".join(lines)\n";

    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("module_a.py"), file_a).unwrap();
    std::fs::write(src.join("module_b.py"), file_b).unwrap();
    std::fs::write(src.join("module_c.py"), file_c).unwrap();

    let opts = ClonesOptions {
        language: Some("python".to_string()),
        min_tokens: 15,
        min_lines: 4,
        threshold: 0.7,
        ..Default::default()
    };

    let report = clones::detect_clones(dir.path(), &opts).unwrap();

    eprintln!(
        "Synthetic test: files_analyzed={}, pairs={}",
        report.stats.files_analyzed,
        report.clone_pairs.len()
    );
    for pair in &report.clone_pairs {
        let func1 = pair.fragment1.function.as_deref().unwrap_or("<none>");
        let func2 = pair.fragment2.function.as_deref().unwrap_or("<none>");
        eprintln!(
            "  Pair {}: {:?} sim={:.4} {:?}:{}..={} [{}] <-> {:?}:{}..={} [{}]",
            pair.id,
            pair.clone_type,
            pair.similarity,
            pair.fragment1.file.file_name().unwrap_or_default(),
            pair.fragment1.start_line,
            pair.fragment1.end_line,
            func1,
            pair.fragment2.file.file_name().unwrap_or_default(),
            pair.fragment2.start_line,
            pair.fragment2.end_line,
            func2,
        );
    }

    assert!(
        report.stats.files_analyzed >= 3,
        "Expected 3 files analyzed, got {}",
        report.stats.files_analyzed
    );

    // We should find at least the exact duplicate (process_data in a and b)
    assert!(
        !report.clone_pairs.is_empty(),
        "Expected at least one clone pair for identical process_data functions, stats: {:?}",
        report.stats
    );

    // Check that an exact or near-exact duplicate is found
    let has_high_sim = report.clone_pairs.iter().any(|p| p.similarity >= 0.95);
    assert!(
        has_high_sim,
        "Expected a high-similarity (>=0.95) clone pair for process_data, best sim={:.4}",
        report
            .clone_pairs
            .iter()
            .map(|p| p.similarity)
            .fold(0.0_f64, f64::max)
    );

    // All previews should be populated
    for pair in &report.clone_pairs {
        assert!(
            pair.fragment1
                .preview
                .as_ref()
                .is_some_and(|p| !p.is_empty()),
            "Pair {}: fragment1 preview is empty",
            pair.id
        );
        assert!(
            pair.fragment2
                .preview
                .as_ref()
                .is_some_and(|p| !p.is_empty()),
            "Pair {}: fragment2 preview is empty",
            pair.id
        );
    }
}

/// Verify that completely unrelated files produce no clone pairs.
#[test]
fn synthetic_unrelated_files_no_clones() {
    let dir = tempfile::TempDir::new().unwrap();

    let file_a = "def fibonacci(n):\n\
                   \x20   if n <= 1:\n\
                   \x20       return n\n\
                   \x20   a, b = 0, 1\n\
                   \x20   for i in range(2, n + 1):\n\
                   \x20       a, b = b, a + b\n\
                   \x20   return b\n\
                   \n\
                   \n\
                   def is_prime(num):\n\
                   \x20   if num < 2:\n\
                   \x20       return False\n\
                   \x20   for i in range(2, int(num ** 0.5) + 1):\n\
                   \x20       if num % i == 0:\n\
                   \x20           return False\n\
                   \x20   return True\n";

    let file_b = "def connect_database(host, port, db_name):\n\
                   \x20   config = {\"host\": host, \"port\": port}\n\
                   \x20   config[\"database\"] = db_name\n\
                   \x20   config[\"timeout\"] = 30\n\
                   \x20   config[\"retries\"] = 3\n\
                   \x20   return config\n\
                   \n\
                   \n\
                   def close_database(connection):\n\
                   \x20   if connection is not None:\n\
                   \x20       connection.commit()\n\
                   \x20       connection.close()\n\
                   \x20       connection = None\n\
                   \x20       return True\n\
                   \x20   return False\n";

    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("math_utils.py"), file_a).unwrap();
    std::fs::write(src.join("db_client.py"), file_b).unwrap();

    let opts = ClonesOptions {
        language: Some("python".to_string()),
        min_tokens: 15,
        min_lines: 4,
        threshold: 0.7,
        ..Default::default()
    };

    let report = clones::detect_clones(dir.path(), &opts).unwrap();

    eprintln!(
        "Unrelated test: files_analyzed={}, pairs={}",
        report.stats.files_analyzed,
        report.clone_pairs.len()
    );
    for pair in &report.clone_pairs {
        eprintln!(
            "  Pair {}: {:?} sim={:.4} {:?}:{}..={} <-> {:?}:{}..={}",
            pair.id,
            pair.clone_type,
            pair.similarity,
            pair.fragment1.file.file_name().unwrap_or_default(),
            pair.fragment1.start_line,
            pair.fragment1.end_line,
            pair.fragment2.file.file_name().unwrap_or_default(),
            pair.fragment2.start_line,
            pair.fragment2.end_line,
        );
    }

    assert_eq!(
        report.clone_pairs.len(),
        0,
        "Unrelated files should produce no clones, but got {}: {:?}",
        report.clone_pairs.len(),
        report
            .clone_pairs
            .iter()
            .map(|p| { format!("Pair {}: {:?} sim={:.3}", p.id, p.clone_type, p.similarity) })
            .collect::<Vec<_>>()
    );
}

// =============================================================================
// Test: Realistic service-layer clones at /tmp/clone-test-corpus
// =============================================================================

#[test]
#[ignore]
fn realistic_service_corpus_diagnostic() {
    let path = Path::new("/tmp/clone-test-corpus");
    if !path.exists() {
        eprintln!("Skipping: /tmp/clone-test-corpus not found");
        return;
    }

    let opts = ClonesOptions {
        min_tokens: 25,
        min_lines: 5,
        threshold: 0.7,
        type_filter: None,
        normalization: NormalizationMode::All,
        language: Some("python".to_string()),
        show_classes: false,
        include_within_file: false,
        max_clones: 100,
        max_files: 1000,
        exclude_generated: false,
        exclude_tests: false,
    };

    let report = clones::detect_clones(path, &opts).unwrap();

    eprintln!("\n=== V2 Realistic Service Corpus ===");
    eprintln!(
        "Files: {}, Tokens: {}",
        report.stats.files_analyzed, report.stats.total_tokens
    );
    eprintln!(
        "Clones: {} (T1:{}, T2:{}, T3:{})",
        report.stats.clones_found,
        report.stats.type1_count,
        report.stats.type2_count,
        report.stats.type3_count
    );
    eprintln!();

    for pair in &report.clone_pairs {
        let f1 = pair
            .fragment1
            .file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let f2 = pair
            .fragment2
            .file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let func1 = pair.fragment1.function.as_deref().unwrap_or("<none>");
        let func2 = pair.fragment2.function.as_deref().unwrap_or("<none>");
        eprintln!(
            "  Pair {:2}: {:?} sim={:.3}  {}:{}-{} [{}] ({:?} tok, {:?} lines)",
            pair.id,
            pair.clone_type,
            pair.similarity,
            f1,
            pair.fragment1.start_line,
            pair.fragment1.end_line,
            func1,
            pair.fragment1.tokens,
            pair.fragment1.lines,
        );
        eprintln!(
            "           {:30}  {}:{}-{} [{}] ({:?} tok, {:?} lines)",
            "",
            f2,
            pair.fragment2.start_line,
            pair.fragment2.end_line,
            func2,
            pair.fragment2.tokens,
            pair.fragment2.lines,
        );
        if let Some(ref preview) = pair.fragment1.preview {
            eprintln!("           frag1: {}", &preview[..preview.len().min(80)]);
        }
        if let Some(ref preview) = pair.fragment2.preview {
            eprintln!("           frag2: {}", &preview[..preview.len().min(80)]);
        }
        eprintln!();
    }

    eprintln!("=== Expected clones ===");
    eprintln!("  T1: validate_user_input <-> validate_order_input (exact copy)");
    eprintln!("  T2: fetch_user_by_id <-> fetch_order_by_id <-> fetch_product_by_id (renamed)");
    eprintln!("  T2: list_users_paginated <-> list_orders_paginated <-> list_products_paginated (renamed)");
    eprintln!("  T3: update_user_profile <-> update_order_status <-> update_product_info (similar + extra logic)");
    eprintln!("=== End ===\n");
}
