# Phased Implementation Plan: clones_v2

Created: 2026-02-18
Author: architect-agent (Opus 4.6)
Phase: 4 of 5 (Phase 3.5 = Prior Art was the last completed)

---

## Overview

This plan breaks the `clones_v2` module rewrite into **6 buildable phases**, each producing
compilable code and progressively passing more of the 65 tests in `clones_v2_tests.rs`.
The module coexists alongside `clones.rs` during transition -- it does NOT replace or modify
`clones.rs` or any of its public re-exports from `mod.rs`.

### File Structure (Final State)

```
crates/tldr-core/src/analysis/
  clones.rs            ← untouched v1 (stays as-is)
  clones_v2/
    mod.rs             ← public API: detect_clones, re-exported types
    types.rs           ← re-export all types from clones.rs (zero duplication)
    extract.rs         ← tree-sitter fragment extraction (REQ-1)
    tokenize.rs        ← tokenization + normalization pipeline
    detect.rs          ← two-phase detection engine (REQ-2, REQ-7)
    similarity.rs      ← Dice coefficient + classify_clone_type
    classes.rs         ← Union-Find clone class computation
    filter.rs          ← is_test_file, is_generated_file, discover_source_files
  clones_v2_tests.rs   ← 65 tests (already written)
```

### Dependency Graph Between Sub-Modules

```
mod.rs (detect_clones entry point)
  ├── types.rs      (re-exports from clones.rs)
  ├── filter.rs     (file discovery)
  ├── tokenize.rs   (AST → token sequences)
  ├── extract.rs    (tree-sitter fragment extraction)
  ├── similarity.rs (Dice, classify)
  ├── detect.rs     (hash bucketing, inverted index, matching)
  └── classes.rs    (Union-Find)
```

---

## Phase 1: Type Skeleton + Module Registration

**Goal:** Make `clones_v2` a valid Rust module that compiles. Re-export all public types
from `clones.rs` so that `clones_v2_tests.rs` can reference `crate::analysis::clones_v2::*`
without duplicating any type definitions.

### Files to Create

#### `clones_v2/mod.rs`

```rust
//! Clone detection v2 -- rewrite with tree-sitter fragment extraction,
//! two-phase hash detection, and correct line numbers.

// Re-export all types from v1 (zero duplication)
mod types;
pub use types::*;

// Stub: the main entry point
pub fn detect_clones(
    path: &std::path::Path,
    options: &ClonesOptions,
) -> anyhow::Result<ClonesReport> {
    // Phase 1 stub: returns empty report
    let start = std::time::Instant::now();
    Ok(ClonesReport {
        root: path.to_path_buf(),
        language: options.language.clone().unwrap_or_else(|| "auto".to_string()),
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
    })
}
```

#### `clones_v2/types.rs`

```rust
//! Re-export all public types from clones.rs.
//! This file exists solely to avoid duplicating type definitions.

pub use crate::analysis::clones::{
    // Report types
    ClonesReport, ClonePair, CloneFragment, CloneType, CloneClass,
    CloneStats, CloneConfig, NormalizationMode, ClonesOptions,
    // Token types
    NormalizedToken, TokenCategory, TokenSequence,
    // Hash types
    RollingHash, HashEntry, HashIndex,
    // Union-Find
    UnionFind,
    // Public functions re-exported for API compatibility
    classify_clone_type, compute_dice_similarity, compute_rolling_hashes,
    hash_token, interpret_similarity, normalize_tokens,
    // Filter functions
    is_test_file, is_generated_file,
};
```

### Files to Modify

#### `analysis/mod.rs`

Add after `pub mod clones;`:
```rust
pub mod clones_v2;
```

Add after the `clones_v2_tests` registration (at the bottom, in the `#[cfg(test)]` block):
```rust
#[cfg(test)]
mod clones_v2_tests;
```

### Tests That Should Pass After Phase 1

**Module: `json_serialization`** (8 tests) -- these only construct types and serialize:
- `test_clones_report_serialization_format`
- `test_clone_type_serde_renames`
- `test_normalization_mode_serde`
- `test_optional_fields_absent_not_null`
- `test_interpretation_absent_when_none`
- `test_class_count_absent_when_none`
- `test_type_filter_absent_when_none`
- `test_full_report_round_trip`

**Module: `preserved_behaviors`** (15 tests) -- these test static functions and defaults:
- `test_is_test_file_directory_patterns`
- `test_is_test_file_name_patterns`
- `test_is_test_file_non_test_files`
- `test_is_generated_file_directory_patterns`
- `test_is_generated_file_suffix_patterns`
- `test_is_generated_file_prefix_patterns`
- `test_is_generated_file_non_generated`
- `test_clones_options_defaults`
- `test_clones_options_new_equals_default`
- `test_classify_clone_type_type1`
- `test_classify_clone_type_type2`
- `test_classify_clone_type_type3`
- `test_clone_type_as_str`
- `test_clone_type_min_similarity`
- `test_clone_type_display`
- `test_normalization_mode_as_str`
- `test_normalization_mode_from_str`
- `test_normalization_mode_flags`
- `test_normalization_mode_default`

**Module: `edge_cases`** (partial -- those that expect empty results):
- `test_empty_directory_returns_empty_report`
- `test_single_file_no_within_file`
- `test_file_below_min_tokens_no_fragments`
- `test_config_snapshot`
- `test_no_panic_on_empty_files`

**Target: ~28 tests passing** (out of 65)

### Risks Addressed
- None yet (skeleton only)

### Implementation Notes
- The `types.rs` re-export approach means zero code duplication. All types live in `clones.rs`.
- `detect_clones` is a stub returning an empty report. This satisfies tests that expect
  empty results for edge cases (empty dir, single file, below-threshold).
- Do NOT modify `clones.rs` at all.
- The `ClonesOptions` default values are `min_tokens=25` and `min_lines=5` (matching v1 exactly).
  The test `test_clones_options_defaults` verifies this.

### Estimated Effort: Small (1-2 hours)

---

## Phase 2: File Discovery + Tokenization + Fragment Extraction

**Goal:** Implement the pipeline from source files to `Vec<TokenSequence>` fragments with
**real line numbers** from tree-sitter. This is the core infrastructure that all detection
phases depend on.

### Files to Create

#### `clones_v2/filter.rs`

Delegate to `clones.rs` functions where possible. New function:

```rust
/// Discover source files for clone detection.
/// Wraps walkdir with extension filter, max_files cap, test/generated exclusion.
pub fn discover_source_files(
    path: &Path,
    language: Option<&str>,
    max_files: usize,
    exclude_generated: bool,
    exclude_tests: bool,
) -> Vec<PathBuf> { ... }

// Re-export from clones.rs:
pub use crate::analysis::clones::{is_test_file, is_generated_file};

// Internal: reuse clones::is_source_file_for_clones or reimplement
fn is_source_file_for_clones(path: &Path, language: Option<&str>) -> bool { ... }
```

#### `clones_v2/tokenize.rs`

Wraps existing tokenization but preserves raw + normalized tokens:

```rust
/// Per-file tokenization result. Stores BOTH raw and normalized tokens.
pub struct FileTokens {
    pub file: PathBuf,
    pub source: String,               // original source (for preview extraction)
    pub raw_tokens: Vec<NormalizedToken>,      // before normalization
    pub normalized_tokens: Vec<NormalizedToken>, // after normalization
}

/// Tokenize a single file. Returns raw tokens from tree-sitter AST walk.
/// Skips comment nodes, import/use nodes, decorator/annotation nodes.
pub fn tokenize_file_v2(path: &Path, language: &str) -> anyhow::Result<FileTokens> { ... }
```

Key differences from v1 `tokenize_file`:
- Skip comment nodes (same as v1)
- **NEW: Skip import/use statement nodes** (RISK-4 mitigation for imports)
- **NEW: Skip decorator/annotation nodes** (RISK-4 mitigation)
- Store `source` text for later preview extraction (REQ-4)
- Return both raw and normalized token vectors

#### `clones_v2/extract.rs`

The critical new module -- tree-sitter query-based fragment extraction:

```rust
/// Extract syntactic fragments from a file using tree-sitter queries.
///
/// Returns fragments aligned to function/method boundaries (REQ-1).
/// Falls back to sliding window if < 2 fragments extracted.
pub fn extract_fragments(
    file_tokens: &FileTokens,
    file_idx: usize,
    min_tokens: usize,
    min_lines: usize,
    normalization: NormalizationMode,
) -> Vec<FragmentData> { ... }

/// Internal: fragment data before conversion to TokenSequence
pub struct FragmentData {
    pub file_idx: usize,
    pub file: PathBuf,
    pub start_line: usize,  // from tree-sitter node.start_position().row + 1
    pub end_line: usize,    // from tree-sitter node.end_position().row + 1
    pub raw_tokens: Vec<NormalizedToken>,
    pub normalized_tokens: Vec<NormalizedToken>,
    pub raw_hash: u64,       // hash of raw token sequence (for Type-1)
    pub normalized_hash: u64, // hash of normalized token sequence (for Type-2)
    pub preview: String,     // source lines for this fragment
    pub function_name: Option<String>,
}
```

Tree-sitter queries per language (RISK-5, RISK-11 addressed):

| Language | Query | Notes |
|----------|-------|-------|
| Python | `(function_definition) @fn` | Top-level only; skip nested via depth check |
| TypeScript/JS | `(function_declaration) @fn`, `(method_definition) @method`, `(arrow_function) @fn` | Skip if parent is another function |
| Go | `(function_declaration) @fn`, `(method_declaration) @method` | Both free functions and methods |
| Rust | `(function_item) @fn` | Extract methods INSIDE impl blocks as individual fns, NOT the impl itself |
| Java | `(method_declaration) @method`, `(constructor_declaration) @ctor` | Skip class-level extraction |

**For each fragment node:**
1. Get `start_line = node.start_position().row + 1` (REQ-3, BUG-1 fix)
2. Get `end_line = node.end_position().row + 1` (REQ-3, BUG-1 fix)
3. Check `end_line - start_line + 1 >= min_lines` (REQ-6, BUG-2 fix)
4. Walk node children to extract tokens, skipping decorators/annotations (RISK-4)
5. Check `tokens.len() >= min_tokens`
6. Compute both `raw_hash` and `normalized_hash` via `RollingHash`
7. Extract preview from source text (REQ-4, BUG-5 fix)
8. Extract function name from the `name` child node

**Decorator skipping (RISK-4):**
For Python `function_definition`, the `decorator` child nodes precede the `def` keyword.
When computing `start_line`, use the first non-decorator child's `start_position()`.
When extracting tokens, skip all `decorator` children.

**Nested function exclusion (RISK-5):**
Only extract top-level syntactic units. After tree-sitter query returns matches, filter:
- If a match's range is fully contained within another match's range (same file), discard
  the outer match (keep the more specific inner functions). Actually: keep the inner,
  discard the outer only if we'd produce subsumption. Decision from RISK-12: do NOT merge,
  just skip fragments below min_tokens.

**Impl block handling (RISK-11):**
For Rust, query `(function_item) @fn` captures both free functions and methods inside
`impl` blocks. Do NOT query `(impl_item)` at all. The methods are already captured.

**Sliding window fallback:**
If tree-sitter queries yield < 2 fragments for a file, fall back to sliding window with:
- `window_size = min_tokens`
- `step = min_tokens / 2`
- `max_fragments_per_file = 200` (RISK-2 cap)
- Line numbers from first/last token's tree-sitter position

**Dedup key (RISK-14):** Use `(file, start_line, end_line)` to deduplicate fragments.

### Files to Modify

#### `clones_v2/mod.rs`

Replace the stub `detect_clones` with a real pipeline up through fragment extraction,
but still return empty `clone_pairs` (detection comes in Phase 3):

```rust
pub fn detect_clones(path: &Path, options: &ClonesOptions) -> anyhow::Result<ClonesReport> {
    let start = Instant::now();

    // Step 1: Discover files
    let files = filter::discover_source_files(
        path, options.language.as_deref(), options.max_files,
        options.exclude_generated, options.exclude_tests,
    );

    // Step 2: Tokenize files
    let file_tokens: Vec<FileTokens> = files.iter()
        .filter_map(|f| tokenize::tokenize_file_v2(f, &lang).ok())
        .collect();

    // Step 3: Extract fragments
    let all_fragments: Vec<FragmentData> = file_tokens.iter().enumerate()
        .flat_map(|(idx, ft)| extract::extract_fragments(
            ft, idx, options.min_tokens, options.min_lines, options.normalization,
        ))
        .collect();

    let total_tokens: usize = file_tokens.iter()
        .map(|ft| ft.raw_tokens.len())
        .sum();

    // Step 4: Detection (stub -- Phase 3)
    let clone_pairs = vec![];

    Ok(ClonesReport {
        root: path.to_path_buf(),
        language: options.language.clone().unwrap_or("auto".to_string()),
        clone_pairs,
        clone_classes: vec![],
        stats: CloneStats {
            files_analyzed: files.len(),
            total_tokens,
            clones_found: 0,
            type1_count: 0, type2_count: 0, type3_count: 0,
            class_count: None,
            detection_time_ms: start.elapsed().as_millis() as u64,
        },
        config: CloneConfig { ... },
    })
}
```

### Tests That Should NEWLY Pass After Phase 2

**Module: `edge_cases`** (additional):
- `test_exclude_tests_option` -- now discover_source_files filters test files
- `test_exclude_generated_option` -- now discover_source_files filters generated files
- `test_stats_consistency` -- stats.files_analyzed is now correct (even if 0 pairs)

**Module: `no_false_positives`** (4 tests) -- these expect 0 clone pairs for different functions:
- `test_keyword_overlap_below_threshold` -- returns 0 pairs (detection stub)
- `test_import_pattern_not_clones` -- returns 0 pairs
- `test_different_structure_same_keywords_not_clones` -- returns 0 pairs
- `test_unrelated_functions_no_match` -- returns 0 pairs

*Note: These pass because detection is still stubbed (returns 0 pairs). They will be
re-validated in Phase 3 when detection is live -- they MUST still pass (no false positives).*

**Target: ~35 tests passing** (cumulative)

### Risks Addressed
- RISK-4 (HIGH): Decorator/annotation skipping in tokenization
- RISK-5 (HIGH): Nested function exclusion in extraction
- RISK-11 (HIGH): Rust impl_item NOT extracted; methods extracted individually
- RISK-12 (HIGH): No merge of adjacent siblings; skip small fragments instead
- RISK-14 (HIGH): Dedup key uses (file, start_line, end_line)
- RISK-2 (HIGH): max_fragments_per_file=200 cap on sliding window fallback
- BUG-1: Real line numbers from tree-sitter
- BUG-2: min_lines enforced in extract_fragments
- BUG-5: Preview populated from source text
- BUG-6: Function-level fragment boundaries

### Estimated Effort: Large (6-10 hours)

---

## Phase 3: Type-1 and Type-2 Detection

**Goal:** Implement hash-bucket matching for Type-1 (raw hash) and Type-2 (normalized hash)
clones. This is the core detection engine that makes most tests pass.

### Files to Create

#### `clones_v2/similarity.rs`

```rust
/// Compute Dice coefficient on token multisets.
/// Uses RAW token values (not normalized) to avoid BUG-4 false positives.
///
/// Both empty -> 1.0
/// One empty -> 0.0
/// Otherwise: 2 * |intersection| / (|a| + |b|)
pub fn compute_dice_similarity(
    tokens_a: &[NormalizedToken],
    tokens_b: &[NormalizedToken],
) -> f64 { ... }

/// Classify similarity score into clone type.
/// Type-1: |sim - 1.0| < 1e-9
/// Type-2: sim >= 0.9 - 1e-9
/// Type-3: sim < 0.9
pub fn classify_clone_type(similarity: f64) -> CloneType { ... }

/// Human-readable similarity interpretation.
pub fn interpret_similarity(similarity: f64) -> String { ... }
```

Note: `compute_dice_similarity` and `classify_clone_type` can delegate to the
`clones.rs` implementations since the logic is identical. Or re-implement for
independence. The key difference: for Type-3 Phase (Phase 4), Dice is computed on
**raw** tokens, not normalized.

#### `clones_v2/detect.rs`

The two-phase detection engine:

```rust
/// Phase 1: Hash-bucket detection for Type-1 and Type-2 clones.
///
/// Algorithm (REQ-2):
/// 1. Build HashMap<u64, Vec<usize>> mapping raw_hash -> fragment indices
/// 2. For each bucket with 2+ fragments: these are Type-1 candidates
///    - Verify with raw Dice similarity
///    - If sim >= 0.99 (epsilon): Type-1
/// 3. Build HashMap<u64, Vec<usize>> mapping normalized_hash -> fragment indices
/// 4. For each bucket with 2+ fragments NOT already found in step 2: Type-2 candidates
///    - Verify with normalized Dice similarity
///    - If sim >= 0.9: Type-2
///
/// Returns Vec<ClonePair>.
pub fn detect_type1_type2(
    fragments: &[FragmentData],
    files: &[PathBuf],
    options: &ClonesOptions,
    found_pairs: &mut HashSet<(PathBuf, usize, usize, PathBuf, usize, usize)>,
) -> Vec<ClonePair> { ... }
```

**Same-file exclusion (REQ-5, BUG-3 fix):**
```rust
// v2: unconditional skip when include_within_file = false
if !options.include_within_file && frag_a.file_idx == frag_b.file_idx {
    continue; // skip ALL same-file pairs
}
// Always skip same-file overlapping pairs
if frag_a.file_idx == frag_b.file_idx {
    if ranges_overlap(frag_a.start_line, frag_a.end_line,
                      frag_b.start_line, frag_b.end_line) {
        continue;
    }
}
```

**Fragment-to-CloneFragment conversion:**
```rust
let fragment = CloneFragment::new(
    frag.file.clone(),
    frag.start_line,    // real tree-sitter line (BUG-1 fix)
    frag.end_line,      // real tree-sitter line (BUG-1 fix)
    frag.raw_tokens.len(),
)
.with_preview(frag.preview.clone())       // BUG-5 fix
.with_function(frag.function_name.clone().unwrap_or_default()); // optional
```

**Pair canonicalization:**
Every pair goes through `ClonePair::new(...).canonical()` to ensure `fragment1.file <= fragment2.file`.

**max_clones enforcement:**
Break detection loop when `clone_pairs.len() >= options.max_clones`.

### Files to Modify

#### `clones_v2/mod.rs`

Wire up detection:

```rust
// Step 4: Type-1 and Type-2 detection
let mut found_pairs = HashSet::new();
let mut clone_pairs = detect::detect_type1_type2(
    &all_fragments, &files, options, &mut found_pairs,
);

// Count by type
let type1_count = clone_pairs.iter().filter(|p| p.clone_type == CloneType::Type1).count();
let type2_count = clone_pairs.iter().filter(|p| p.clone_type == CloneType::Type2).count();

// Assign sequential 1-indexed IDs
for (i, pair) in clone_pairs.iter_mut().enumerate() {
    pair.id = i + 1;
}
```

### Tests That Should NEWLY Pass After Phase 3

**Module: `accurate_line_numbers`** (3 tests):
- `test_python_function_line_numbers_match_tree_sitter`
- `test_rust_impl_block_method_line_ranges`
- `test_line_numbers_not_derived_from_token_count`

**Module: `function_level_extraction`** (4 tests):
- `test_identical_functions_detected_as_clone`
- `test_different_functions_not_detected`
- `test_fragment_boundaries_are_syntactic`
- `test_renamed_identifiers_detected_as_type2`

**Module: `no_false_positives`** (4 tests) -- must STILL pass with live detection:
- `test_keyword_overlap_below_threshold`
- `test_import_pattern_not_clones`
- `test_different_structure_same_keywords_not_clones`
- `test_unrelated_functions_no_match`

**Module: `function_level_extraction`**:
- `test_import_differences_not_false_positive`

**Module: `preview_populated`** (3 tests):
- `test_all_fragments_have_preview`
- `test_preview_contains_source_code`
- `test_preview_truncated_to_100_chars`

**Module: `include_within_file`** (3 tests):
- `test_within_file_false_excludes_all_same_file`
- `test_within_file_true_includes_same_file_pairs`
- `test_overlapping_same_file_always_excluded`

**Module: `min_lines_enforced`** (4 tests):
- `test_min_lines_excludes_short_fragments`
- `test_3_line_clone_not_reported_with_min_lines_5`
- `test_exactly_min_lines_included`
- `test_all_fragments_respect_min_lines`

**Module: `sequence_matching`** (partial -- Type-1/2 tests):
- `test_identical_sequences_type1`
- `test_renamed_identifiers_type2`
- `test_completely_different_no_match`

**Module: `edge_cases`** (additional):
- `test_max_clones_limit`
- `test_stats_consistency`
- `test_clone_pair_ids_sequential`
- `test_canonical_pair_ordering`

**Target: ~55-58 tests passing** (cumulative)

### Risks Addressed
- BUG-3 (include_within_file): Unconditional same-file skip when flag is false
- BUG-4 (false positives): Raw tokens used for Dice similarity, not normalized
- RISK-15: Type-2 detection via normalized hash catches [0.9, 1.0) pairs that v1 missed
- RISK-1 (partial): Raw tokens used for primary hashing prevents $ID collapse

### Estimated Effort: Large (6-10 hours)

---

## Phase 4: Type-3 Detection (Inverted Index)

**Goal:** Implement Type-3 (gapped clone) detection using inverted index on raw token
values with Dice verification. This completes the detection pipeline.

### Files to Modify

#### `clones_v2/detect.rs`

Add Type-3 detection function:

```rust
/// Phase 2: Inverted-index detection for Type-3 clones.
///
/// Algorithm (REQ-7, SourcererCC-inspired):
/// 1. Build inverted index: raw_token_value -> Vec<fragment_idx>
/// 2. Cap posting lists at MAX_POSTING_LIST_SIZE (500) -- RISK-1 mitigation
/// 3. For each fragment pair appearing in 2+ posting lists:
///    count shared tokens (posting list intersection)
/// 4. If estimated overlap >= threshold * 0.5 (pre-filter):
///    compute full Dice similarity on raw tokens
/// 5. If Dice >= threshold AND pair not already found: emit Type-3
///
/// RISK-1 critical fix: The inverted index uses RAW token values.
/// With normalization=All, $ID would appear in every fragment, creating
/// O(n^2) pairs. Raw tokens have natural diversity.
///
/// RISK-15 fix: No similarity >= 0.9 filter. Type-3 phase reports ALL
/// pairs above threshold that weren't caught by Type-1/2 hash matching.
pub fn detect_type3(
    fragments: &[FragmentData],
    files: &[PathBuf],
    options: &ClonesOptions,
    found_pairs: &mut HashSet<(PathBuf, usize, usize, PathBuf, usize, usize)>,
    current_count: usize,
) -> Vec<ClonePair> { ... }
```

**Posting list cap (RISK-1 CRITICAL):**
```rust
const MAX_POSTING_LIST_SIZE: usize = 500;

// When building inverted index:
for (token_value, fragment_indices) in &inverted_index {
    if fragment_indices.len() > MAX_POSTING_LIST_SIZE {
        continue; // Skip non-discriminative tokens ($ID-like in practice)
    }
    // ... generate candidate pairs from this posting list
}
```

**Type-3 skip conditions (preserved from v1):**
- Skip entirely if `type_filter` is `Some(Type1)` or `Some(Type2)`
- Skip if `max_clones` already reached after Type-1/2

**RISK-15 fix:** Do NOT filter `similarity >= 0.9` in Type-3 phase. Instead, check
against `found_pairs` HashSet to avoid duplicate reporting. If a pair was already
found by Type-1/2 hash matching, skip it. Otherwise report it even if similarity is 0.95.

#### `clones_v2/mod.rs`

Wire up Type-3 after Type-1/2:

```rust
// Step 5: Type-3 detection (if applicable)
let skip_type3 = matches!(options.type_filter, Some(CloneType::Type1) | Some(CloneType::Type2))
    || clone_pairs.len() >= options.max_clones;

if !skip_type3 {
    let type3_pairs = detect::detect_type3(
        &all_fragments, &files, options, &mut found_pairs, clone_pairs.len(),
    );
    clone_pairs.extend(type3_pairs);
}

// Re-assign IDs after combining
for (i, pair) in clone_pairs.iter_mut().enumerate() {
    pair.id = i + 1;
}

let type3_count = clone_pairs.iter().filter(|p| p.clone_type == CloneType::Type3).count();
```

### Tests That Should NEWLY Pass After Phase 4

**Module: `sequence_matching`** (remaining Type-3 tests):
- `test_gapped_sequences_type3`
- `test_type3_logging_augmented`

**Target: ~60 tests passing** (cumulative)

### Risks Addressed
- RISK-1 (CRITICAL): Raw token inverted index + posting list cap at 500
- RISK-15 (MEDIUM): No similarity >= 0.9 filter in Type-3 phase

### Implementation Notes

The Type-3 inverted index is the most performance-critical code path. Key optimizations:
1. **Pre-filter with token overlap count.** Before computing full Dice, count how many
   shared posting lists two fragments appear in. If `shared_tokens / max(|A|, |B|) < threshold * 0.5`,
   skip (SourcererCC early termination).
2. **Candidate pair dedup.** Use a HashSet to avoid processing the same (i, j) pair
   from multiple posting lists.
3. **Break on max_clones.** Check after each verified pair.

### Estimated Effort: Medium (4-6 hours)

---

## Phase 5: Clone Classes (Union-Find)

**Goal:** Implement `compute_clone_classes` for the `show_classes=true` option.

### Files to Create

#### `clones_v2/classes.rs`

```rust
/// Compute clone classes from clone pairs using Union-Find.
///
/// Algorithm: same as v1 -- path-compressed union by rank.
/// Groups transitive pairs into classes. Excludes classes with < 2 members.
///
/// Returns Vec<CloneClass> with 1-indexed IDs.
pub fn compute_clone_classes(pairs: &[ClonePair]) -> Vec<CloneClass> { ... }
```

Can delegate to `clones::UnionFind` (re-exported via types.rs) or reimplement.
The algorithm is straightforward Union-Find.

### Files to Modify

#### `clones_v2/mod.rs`

Add class computation:

```rust
// Step 6: Clone classes (if requested)
let clone_classes = if options.show_classes {
    let classes = classes::compute_clone_classes(&clone_pairs);
    stats.class_count = Some(classes.len());
    classes
} else {
    vec![]
};
```

### Tests That Should NEWLY Pass After Phase 5

No new tests specifically -- clone classes are not tested in the v2 test suite yet.
However, this ensures `show_classes=true` works correctly for integration testing.

**Target: ~60 tests passing** (same -- no new test coverage for classes)

### Risks Addressed
- RISK-18 (LOW): CloneFragment equality for Union-Find keying. Use `(file, start_line, end_line)`
  as the fragment identity, not the derived `PartialEq` which includes `preview`.

### Estimated Effort: Small (1-2 hours)

---

## Phase 6: Integration Polish + All Tests Green

**Goal:** Fix any remaining test failures, ensure all 65 tests pass, handle edge cases,
and verify end-to-end correctness.

### Tasks

1. **Run all 65 tests and triage failures.** At this point, most tests should pass.
   Fix any that don't by tracing through the specific test's fixture data and
   expected assertions.

2. **Edge case hardening:**
   - `test_no_panic_on_empty_files` -- ensure empty/comment-only/whitespace-only files
     don't panic during tokenization or extraction
   - `test_file_below_min_tokens_no_fragments` -- `def f(): pass` has ~4 tokens; with
     `min_tokens=25` it should produce zero fragments

3. **Stats consistency audit:**
   - `clones_found == type1_count + type2_count + type3_count`
   - `clones_found == clone_pairs.len()`
   - `files_analyzed` reflects actual files processed (not filtered out)
   - `total_tokens` is sum of raw tokens across all files

4. **Preview correctness:**
   - Verify `preview` contains actual `def` keyword for Python functions
   - Verify truncation to 100 chars with `...` suffix

5. **Canonical ordering verification:**
   - All pairs have `fragment1.file <= fragment2.file`
   - Same-file pairs have `fragment1.start_line <= fragment2.start_line`

6. **Performance sanity check:**
   - Run against a medium codebase (~100 files) and verify it completes in < 10s
   - Monitor memory usage with large files to verify RISK-2 cap works

7. **False positive regression test:**
   - Ensure `test_keyword_overlap_below_threshold` passes with live Type-3 detection
   - This is the critical BUG-4 regression test

### Tests That Should NEWLY Pass After Phase 6

All remaining tests from any module that failed in prior phases due to edge cases
or integration issues. Target: **65/65 tests green.**

### Files to Modify

Potentially any `clones_v2/*.rs` file for bug fixes discovered during test runs.

### Estimated Effort: Medium (3-5 hours)

---

## Summary: Phase-to-Test Mapping

| Phase | Focus | New Tests Passing | Cumulative | Key Bugs Fixed |
|-------|-------|-------------------|------------|----------------|
| 1 | Type skeleton + module registration | ~28 | ~28 | None (skeleton) |
| 2 | File discovery + tokenization + extraction | ~7 | ~35 | BUG-1, BUG-2, BUG-5, BUG-6 |
| 3 | Type-1/2 hash detection | ~20-23 | ~55-58 | BUG-3, BUG-4 |
| 4 | Type-3 inverted index detection | ~2 | ~60 | RISK-1, RISK-15 |
| 5 | Clone classes (Union-Find) | 0 | ~60 | RISK-18 |
| 6 | Integration polish | ~5 | 65 | All remaining |

## Summary: Risk Mitigation Schedule

| Risk | Phase | Mitigation |
|------|-------|------------|
| RISK-1 (CRITICAL) | Phase 4 | Raw token inverted index + posting list cap 500 |
| RISK-2 (HIGH) | Phase 2 | max_fragments_per_file=200, max_file_tokens=50000 |
| RISK-4 (HIGH) | Phase 2 | Skip decorator/annotation nodes during extraction |
| RISK-5 (HIGH) | Phase 2 | Top-level syntactic units only |
| RISK-11 (HIGH) | Phase 2 | Extract fn_item inside impl, not impl itself |
| RISK-12 (HIGH) | Phase 2 | No merge; skip small fragments |
| RISK-14 (HIGH) | Phase 2 | Dedup key = (file, start_line, end_line) |
| RISK-15 (MEDIUM) | Phase 4 | No similarity >= 0.9 filter in Type-3 |
| RISK-18 (LOW) | Phase 5 | Fragment identity by location, not by content |

## Summary: Bug Fix Schedule

| Bug | Phase | Fix |
|-----|-------|-----|
| BUG-1 (fabricated lines) | Phase 2 | node.start_position().row + 1 |
| BUG-2 (min_lines ignored) | Phase 2 | enforce line_count >= min_lines |
| BUG-3 (within_file logic) | Phase 3 | unconditional skip when flag false |
| BUG-4 (false positives) | Phase 3 | raw tokens for Dice similarity |
| BUG-5 (preview empty) | Phase 2 | with_preview() from source lines |
| BUG-6 (fixed windows) | Phase 2 | tree-sitter query extraction |

## Open Questions

1. **Should `types.rs` re-export or copy types?** Plan assumes re-export from `clones.rs`.
   This means `clones_v2` depends on `clones.rs` staying unchanged. If we later want to
   remove `clones.rs`, we'd move the types to `clones_v2/types.rs` and have `clones.rs`
   re-export from `clones_v2` instead.

2. **Sliding window token-level line numbers.** The fallback sliding window needs per-token
   line numbers. Since `NormalizedToken` does not store position, we need to either:
   (a) extend `NormalizedToken` with a `line: usize` field, or
   (b) maintain a parallel `Vec<usize>` of line numbers alongside the token vector, or
   (c) use the tree-sitter node positions stored separately.
   Option (b) is simplest and avoids changing the shared type.

3. **`NormalizationMode::None` and Type-2 detection.** Per RISK-17, should v2 still compute
   a normalized hash for Type-2 even when the user sets `--normalize none`? The spec (REQ-2)
   implies yes -- normalized hashes are always computed internally. The `normalization` option
   only controls whether the *reported similarity* uses raw or normalized values.
   Decision: Always compute both hashes. `normalization` controls the Dice comparison display
   but not the internal hashing.

## Success Criteria

1. All 65 tests in `clones_v2_tests.rs` pass
2. Zero clone pairs reported for structurally different functions (false positive rate = 0%)
3. Line numbers match tree-sitter positions (never derived from token counts)
4. Preview field populated for all fragments
5. min_lines parameter enforced (3-line functions excluded when min_lines=5)
6. include_within_file=false excludes ALL same-file pairs
7. JSON serialization identical to v1 (same field names, same serde rules)
8. No modifications to `clones.rs` or its public API
9. Detection completes in < 5 seconds for a 100-file codebase

