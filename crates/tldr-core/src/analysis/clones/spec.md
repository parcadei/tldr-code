# Clone Detection v2 Specification

Generated: 2026-02-18  
Source analyzed: `crates/tldr-core/src/analysis/clones.rs` (2201 lines)  
CLI analyzed: `crates/tldr-cli/src/commands/clones.rs` (157 lines)

---

## Summary

The clone detection module (`detect_clones`) finds duplicated code fragments across a
directory tree using a three-phase algorithm:

1. **File discovery** via walkdir with extension-based language filtering
2. **Tokenization** via tree-sitter AST extraction followed by normalization
3. **Detection** via hash-index matching (Type-1/2) + inverted-index (Type-3), followed by
   Dice coefficient verification

The current implementation has **six known bugs** documented below. v2 must fix all six
while preserving the public type hierarchy and JSON serialization format exactly.

---

## 1. Public API Surface

### Entry Point

```rust
pub fn detect_clones(path: &Path, options: &ClonesOptions) -> anyhow::Result<ClonesReport>
```

This is the **only** public API called by the CLI. All other public symbols are exposed for
testing or internal use.

### Public Functions

| Function | Signature | Purpose |
|---|---|---|
| `detect_clones` | `(&Path, &ClonesOptions) -> anyhow::Result<ClonesReport>` | Main entry point |
| `normalize_tokens` | `(&str, &str, NormalizationMode) -> anyhow::Result<Vec<NormalizedToken>>` | Tokenize + normalize source string |
| `compute_rolling_hashes` | `(&[NormalizedToken], usize) -> Vec<(u64, usize)>` | Rabin-Karp rolling hashes |
| `hash_token` | `(&NormalizedToken) -> u64` | Hash a single token |
| `has_parse_errors` | `(&Tree) -> bool` | >50% ERROR nodes check |
| `tokenize_file` | `(&Path, &str) -> anyhow::Result<Vec<NormalizedToken>>` | Parse file to raw tokens |
| `extract_tokens_from_ast` | `(&Tree, &[u8], &str) -> Vec<NormalizedToken>` | AST walk to token list |
| `compute_dice_similarity` | `(&[NormalizedToken], &[NormalizedToken]) -> f64` | Dice coefficient on multisets |
| `verify_clone_match` | `(&[NormalizedToken], &[NormalizedToken], f64) -> Option<f64>` | Threshold-filtered similarity |
| `find_verified_clones` | `(&HashIndex, &[Vec<TokenSequence>], f64) -> Vec<(usize,usize,usize,usize,f64)>` | Hash candidates + verification |
| `classify_clone_type` | `(f64) -> CloneType` | Similarity score to Type-1/2/3 |
| `interpret_similarity` | `(f64) -> String` | Human-readable score description |
| `apply_normalization` | `(Vec<NormalizedToken>, NormalizationMode) -> Vec<NormalizedToken>` | Apply normalization pass |
| `is_test_file` | `(&Path) -> bool` | Test file pattern check |
| `is_generated_file` | `(&Path) -> bool` | Generated file pattern check |
| `is_comment_node` | `(&str, &str) -> bool` | AST node comment check |
| `categorize_token` | `(&str, &str) -> TokenCategory` | Node kind → TokenCategory |

### Private Functions (internal, not re-exported)

`build_inverted_index`, `find_type3_candidates`, `discover_source_files`,
`is_source_file_for_clones`, `get_language_from_path`, `tokenize_files`,
`extract_fragments`, `compute_fragment_hash`, `create_pair_key`, `ranges_overlap`,
`compute_clone_classes`, `normalize_single_token`, `extract_tokens_recursive`,
`is_whitespace_only`, `should_capture_as_token`,
`categorize_python_token`, `categorize_typescript_token`, `categorize_go_token`,
`categorize_rust_token`, `categorize_java_token`

---

## 2. Type System

### ClonesReport

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClonesReport {
    pub root: PathBuf,               // Root path analyzed
    pub language: String,            // Language filter used, or "auto"
    pub clone_pairs: Vec<ClonePair>, // All detected clone pairs
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clone_classes: Vec<CloneClass>, // Only present when show_classes=true
    pub stats: CloneStats,           // Detection statistics
    pub config: CloneConfig,         // Configuration snapshot
}
```

Default: all fields zeroed/empty.

### ClonePair

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClonePair {
    pub id: usize,                   // 1-indexed, sequential
    pub clone_type: CloneType,       // Type-1 / Type-2 / Type-3
    pub similarity: f64,             // Dice coefficient [0.0, 1.0]
    pub fragment1: CloneFragment,    // Canonical: fragment1.file <= fragment2.file
    pub fragment2: CloneFragment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<String>, // Human-readable similarity label
}
```

Constructor `ClonePair::new(id, clone_type, similarity, f1, f2)` auto-populates
`interpretation`. The `.canonical()` method swaps fragments so `fragment1.file <
fragment2.file` (or `fragment1.start_line < fragment2.start_line` when same file).

### CloneFragment

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct CloneFragment {
    pub file: PathBuf,                // Absolute or relative path
    pub start_line: usize,            // 1-indexed
    pub end_line: usize,              // 1-indexed, inclusive
    pub tokens: usize,                // Token count
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<usize>,         // = end_line - start_line + 1
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,     // Always None in v1 (bug)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,      // Always None in v1 (bug)
}
```

Constructor: `CloneFragment::new(file, start_line, end_line, tokens)` — sets `lines`
automatically. Builder methods: `.with_function(String)`, `.with_preview(String)` (preview
truncated to 100 chars with `...`).

### CloneType

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CloneType {
    #[serde(rename = "Type-1")]  Type1,  // similarity == 1.0 (epsilon 1e-9)
    #[serde(rename = "Type-2")]  Type2,  // 0.9 <= similarity < 1.0
    #[serde(rename = "Type-3")]  Type3,  // 0.7 <= similarity < 0.9
}
```

Serializes as `"Type-1"`, `"Type-2"`, `"Type-3"` in JSON.

Methods:
- `as_str() -> &'static str` — returns the serde-rename string
- `min_similarity() -> f64` — Type1=1.0, Type2=0.9, Type3=0.7
- `Display` — delegates to `as_str()`

### CloneClass

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneClass {
    pub id: usize,                   // 1-indexed
    pub fragments: Vec<CloneFragment>,
    pub size: usize,                 // = fragments.len()
    pub clone_type: CloneType,       // Dominant type by frequency count
    pub avg_similarity: f64,         // Average pairwise similarity within class
}
```

Only populated in `ClonesReport.clone_classes` when `ClonesOptions.show_classes = true`.
Uses Union-Find to merge transitive pairs. Classes with `size < 2` are excluded.

### CloneStats

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CloneStats {
    pub files_analyzed: usize,
    pub total_tokens: usize,
    pub clones_found: usize,         // = type1_count + type2_count + type3_count
    pub type1_count: usize,
    pub type2_count: usize,
    pub type3_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_count: Option<usize>,  // None unless show_classes=true
    pub detection_time_ms: u64,
}
```

### CloneConfig

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneConfig {
    pub min_tokens: usize,           // Default: 25
    pub min_lines: usize,            // Default: 5
    pub similarity_threshold: f64,  // Default: 0.7
    pub normalization: NormalizationMode, // Default: All
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_filter: Option<CloneType>,  // None = all types
}
```

### NormalizationMode

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NormalizationMode {
    None,          // no normalization → "none" in JSON
    Identifiers,   // replace identifiers with $ID → "identifiers"
    Literals,      // replace literals with $STR/$NUM → "literals"
    #[default]
    All,           // both identifiers and literals → "all"
}
```

Methods: `as_str()`, `from_str(&str) -> Option<Self>`, `normalize_identifiers() -> bool`,
`normalize_literals() -> bool`.

### ClonesOptions

```rust
#[derive(Debug, Clone)]
pub struct ClonesOptions {
    pub min_tokens: usize,           // Default: 25 (CLI default: 25)
    pub min_lines: usize,            // Default: 5  (CLI default: 5) — BUG: ignored
    pub threshold: f64,              // Default: 0.7
    pub type_filter: Option<CloneType>,
    pub normalization: NormalizationMode, // Default: All
    pub language: Option<String>,    // None = auto-detect from extension
    pub show_classes: bool,          // Default: false
    pub include_within_file: bool,   // Default: false — BUG: partially broken
    pub max_clones: usize,           // Default: 100 (but CLI default: 20)
    pub max_files: usize,            // Default: 1000
    pub exclude_generated: bool,     // Default: false
    pub exclude_tests: bool,         // Default: false
}
```

Note: `ClonesOptions::new()` and `Default` both yield the same values.

### Internal Types (not serialized)

```rust
pub struct NormalizedToken {
    pub value: String,      // Normalized value (may be $ID, $STR, $NUM)
    pub original: String,   // Original text from source
    pub category: TokenCategory,
}

pub enum TokenCategory {
    Identifier, StringLiteral, NumericLiteral,
    Keyword, Operator, Punctuation, Other,
}

pub struct TokenSequence {
    pub file: PathBuf,
    pub start_line: usize,  // 1-indexed — BUG: fabricated in v1
    pub end_line: usize,    // 1-indexed — BUG: fabricated in v1
    pub tokens: Vec<NormalizedToken>,
    pub hash: u64,
}

pub struct RollingHash { value, window_size, base (31), modulus (1_000_000_007), base_power }
pub struct HashEntry   { hash, file_idx, start_pos, end_pos }
pub struct HashIndex   { index: HashMap<u64, Vec<HashEntry>> }
pub struct UnionFind   { parent: Vec<usize>, rank: Vec<usize> }
```

---

## 3. Behavioral Contracts

### `detect_clones(path, options) -> Result<ClonesReport>`

**Pre:** `path` is a readable directory (or file).  
**Post:** Returns `Ok(ClonesReport)` always; returns empty report (no pairs) on empty dirs or
files with no fragments.  
**Never panics:** all errors in file parsing silently skip the file.  
**max_clones enforcement:** detection halts as soon as `clone_pairs.len() >= max_clones`.  
**Type-3 skip condition:** skipped entirely if `type_filter` excludes Type-3 OR if
`max_clones` is already reached after Type-1/2.

Algorithm steps (verified from source):
1. `discover_source_files(path, language, max_files, exclude_generated, exclude_tests)`
2. `tokenize_files(files, normalization, min_tokens, min_lines)` — returns per-file
   `Vec<TokenSequence>` fragments + total token count
3. Build `HashMap<u64, Vec<usize>>` hash index of fragment hashes
4. For each hash bucket with >= 2 entries: verify pairs with `compute_dice_similarity`,
   skip same-file pairs (if `!include_within_file`) only when they also overlap
5. Type-3: build inverted index, find candidates, verify with Dice
6. If `show_classes`: run `compute_clone_classes` (Union-Find)

### `classify_clone_type(similarity) -> CloneType`

Uses epsilon `1e-9`:
- `|similarity - 1.0| < 1e-9` → Type1
- `similarity >= 0.9 - 1e-9` → Type2
- else → Type3

### `compute_dice_similarity(tokens1, tokens2) -> f64`

- Both empty → `1.0`
- One empty → `0.0`
- Otherwise: `2 * |intersection| / (|tokens1| + |tokens2|)` where intersection is multiset
  intersection (sum of min counts per token value)

### `has_parse_errors(tree) -> bool`

- If `!root.has_error()` → false (fast path)
- Otherwise: count named child nodes with `kind == "ERROR" || is_error()`
- Returns true only if `errors * 2 > total` (i.e., >50% error nodes)
- **Effect:** tolerates partial parse errors (e.g., C preprocessor macros)

### `extract_fragments(file_path, tokens, min_tokens, _min_lines)`

- If `tokens.len() < min_tokens` → empty vec
- If `tokens.len() <= 500` (MAX_SINGLE_FRAGMENT_SIZE): one fragment for entire file
  - `start_line = 1`, `end_line = tokens.len() / 5 + 1` (FABRICATED)
- Else: sliding window of size `min_tokens`, step `min_tokens / 2`
  - `start_line = start / 10 + 1` (FABRICATED)
  - `end_line = end / 10 + 1` (FABRICATED)

### `is_test_file(path) -> bool`

Directory path contains: `/tests/`, `/test/`, `/__tests__/`, `/spec/`, `/testing/`  
File name: `test_*`, `*_test.py`, `*_test.go`, `*_test.rs`, `*_spec.rb`,
`.test.ts`, `.test.js`, `.spec.ts`, `.spec.js`, `*Test.java`, `*Tests.cs`

### `is_generated_file(path) -> bool`

Directories: `vendor/`, `node_modules/`, `__pycache__/`, `dist/`, `build/`, `target/`,
`gen/`, `generated/`, `.gen/`, `third_party/`, `external/`  
Suffixes: `.pb.go`, `_pb2.py`, `.pb.ts`, `.pb.js`, `.pb.rs`, `_grpc.pb.go`, `_pb2_grpc.py`,
`.generated.ts/.tsx/.js`, `.graphql.ts/.tsx`, `_generated.{go,ts,rs,py}`,
`.gen.{go,ts,rs}`, `_mock.go`, `_mocks.go`, `.thrift.go`  
Prefixes (case-insensitive): `generated_`, `auto_generated`, `autogenerated`, `mock_`, `mocks_`

### `is_source_file_for_clones(path, language) -> bool`

With language filter: only matches exact language extension.  
Without filter: matches `.py`, `.ts`, `.tsx`, `.js`, `.jsx`, `.go`, `.rs`, `.java`.  
No support for C, C++, Ruby, Kotlin, Swift, etc.

---

## 4. CLI Integration

File: `crates/tldr-cli/src/commands/clones.rs`

### ClonesArgs → ClonesOptions Mapping

| CLI arg | Default | ClonesOptions field |
|---|---|---|
| `path` (positional) | `.` | passed to `detect_clones(path, ...)` |
| `--min-tokens` | `25` | `min_tokens` |
| `--min-lines` | `5` | `min_lines` (ignored in core — bug) |
| `-t` / `--threshold` | `0.7` | `threshold` |
| `--type-filter` | `"all"` | `type_filter` via `parse_type_filter` |
| `--normalize` | `"all"` | `normalization` via `NormalizationMode::from_str` |
| `--language` | None | `language` |
| `-o` / `--output` | `"json"` | controls output format, not passed to core |
| `--show-classes` | false | `show_classes` |
| `--include-within-file` | false | `include_within_file` |
| `--max-clones` | `20` | `max_clones` (note: core default is 100) |
| `--max-files` | `1000` | `max_files` |
| `--exclude-generated` | false | `exclude_generated` |
| `--exclude-tests` | false | `exclude_tests` |

### parse_type_filter(s: &str) -> Option<CloneType>

- `"1"` → `Some(Type1)`
- `"2"` → `Some(Type2)`
- `"3"` → `Some(Type3)`
- `"all"` or `""` or anything else → `None`

### Output Formats

- `"json"` (default): `writer.write(&report)` — serializes `ClonesReport` as JSON
- `"text"`: `format_clones_text(&report)` from `crate::output`
- `"sarif"`: `format_clones_sarif(&report)` from `crate::output`
- `"dot"`: `format_clones_dot(&report)` from `crate::output`, returns early

---

## 5. Serialization Contracts

### JSON Output Format

The root object is `ClonesReport`:

```json
{
  "root": "/path/to/analyzed/dir",
  "language": "auto",
  "clone_pairs": [
    {
      "id": 1,
      "clone_type": "Type-2",
      "similarity": 0.923,
      "fragment1": {
        "file": "src/a.py",
        "start_line": 10,
        "end_line": 25,
        "tokens": 47,
        "lines": 16
      },
      "fragment2": {
        "file": "src/b.py",
        "start_line": 10,
        "end_line": 25,
        "tokens": 47,
        "lines": 16
      },
      "interpretation": "Very high similarity (Type-1/2 clone)"
    }
  ],
  "stats": {
    "files_analyzed": 12,
    "total_tokens": 4821,
    "clones_found": 3,
    "type1_count": 0,
    "type2_count": 2,
    "type3_count": 1,
    "detection_time_ms": 142
  },
  "config": {
    "min_tokens": 25,
    "min_lines": 5,
    "similarity_threshold": 0.7,
    "normalization": "all"
  }
}
```

### Serde Rules Summary

| Field | Rule |
|---|---|
| `clone_classes` | `skip_serializing_if = "Vec::is_empty"` — absent when empty |
| `clone_pairs[].interpretation` | `skip_serializing_if = "Option::is_none"` |
| `fragment.lines` | `skip_serializing_if = "Option::is_none"` |
| `fragment.function` | `skip_serializing_if = "Option::is_none"` |
| `fragment.preview` | `skip_serializing_if = "Option::is_none"` |
| `stats.class_count` | `skip_serializing_if = "Option::is_none"` |
| `config.type_filter` | `skip_serializing_if = "Option::is_none"` |
| `CloneType` | `rename`: Type1→"Type-1", Type2→"Type-2", Type3→"Type-3" |
| `NormalizationMode` | `rename_all = "lowercase"`: None→"none", All→"all", etc. |

---

## 6. Edge Cases

| Scenario | v1 Behavior |
|---|---|
| Empty directory | Returns `ClonesReport` with zero stats, empty pairs |
| Single file | Returns empty pairs (cannot clone with itself unless `include_within_file=true`) |
| File with parse errors | Silently skipped (logged at tokenize_file level, error swallowed) |
| Binary file | tree-sitter parse fails → skipped silently |
| File < `min_tokens` tokens | `extract_fragments` returns empty → file contributes no fragments |
| File 1..=500 tokens | One whole-file fragment |
| File > 500 tokens | Sliding windows of size `min_tokens`, step `min_tokens/2` |
| `max_files` exceeded | WalkDir breaks after collecting `max_files` entries |
| `max_clones` exceeded | Detection loop breaks; Type-3 phase may be skipped entirely |
| Unsupported language | `is_source_file_for_clones` returns false → file skipped |
| `language` filter mismatch | Only files matching exact extension are processed |
| `type_filter = Some(Type1)` | Type-3 phase is entirely skipped |
| Hash collision | `compute_dice_similarity` post-verification rejects non-matches |

---

## 7. Known Bugs in v1 (to Fix in v2)

### BUG-1: Fabricated Line Numbers

**Location:** `extract_fragments` lines 1609-1631  
**Code:**
```rust
// Small file (<=500 tokens):
end_line: tokens.len() / 5 + 1,  // divide token count by 5

// Large file sliding window:
let start_line = start / 10 + 1; // divide token index by 10
let end_line   = end   / 10 + 1;
```
**Problem:** Line numbers are derived from token count/index using an arbitrary divisor,
not from actual source positions. For a file with 250 tokens but 50 lines, the reported
`end_line` would be 51 but real end is 50. For a 1000-token file, windows would report
`start_line = 1` for tokens 0-24, which may span real lines 1-8.

**v2 Fix:** Use `node.start_position().row + 1` and `node.end_position().row + 1` from the
tree-sitter `Node` at fragment boundaries.

### BUG-2: min_lines Parameter is Ignored

**Location:** `extract_fragments` signature, line 1591  
**Code:**
```rust
fn extract_fragments(
    file_path: &Path,
    tokens: &[NormalizedToken],
    min_tokens: usize,
    _min_lines: usize,   // ← prefixed with _ = intentionally ignored
) -> Vec<TokenSequence>
```
**Problem:** The `min_lines` check is never applied. A 1-line fragment with 25 tokens
passes through even if `min_lines = 5`.

**v2 Fix:** Enforce `fragment.line_count() >= min_lines` before adding to results.

### BUG-3: include_within_file Logic Error

**Location:** `detect_clones` lines 1113-1118 and 1202-1206  
**Code:**
```rust
if !options.include_within_file && file_idx1 == file_idx2 {
    // Also check for overlapping regions
    if ranges_overlap(...) {
        continue;  // ← only skips OVERLAPPING same-file pairs
    }
    // Non-overlapping same-file pairs fall through and ARE included!
}
```
**Problem:** When `include_within_file = false`, non-overlapping same-file pairs (e.g., two
different functions in the same file) are still included. The intent of the flag is to
exclude ALL same-file matches, but only overlapping ones are excluded.

**v2 Fix:** When `!include_within_file && file_idx1 == file_idx2`, skip the pair
unconditionally (remove the inner `ranges_overlap` condition entirely).

### BUG-4: Catastrophic False Positives from Normalization + Bag-of-Tokens

**Location:** `normalize_single_token` + `compute_dice_similarity`  
**Problem:** When `normalization = All` (default), ALL identifiers become `$ID` and all
strings become `$STR`. Two completely different files with lots of variable assignments both
become bags of `{ "$ID": N, "=": M, ... }`. The Dice coefficient on these bags can be very
high (0.8+) even for unrelated code. Example: any two files that heavily use assignment
statements will appear as Type-3 clones.

**v2 Fix:** Use raw (un-normalized) token sequences for Rabin-Karp Type-1/2 hash matching.
Apply normalization only for Type-2 candidate confirmation, not for the primary hash or
the Dice similarity comparison.

### BUG-5: CloneFragment.preview Never Populated

**Location:** `detect_clones` lines 1144-1154  
**Code:**
```rust
let fragment1 = CloneFragment::new(
    files[*file_idx1].clone(),
    frag1.start_line,
    frag1.end_line,
    frag1.tokens.len(),
);
// No .with_preview() call
```
**Problem:** `CloneFragment` has a `with_preview(String)` builder method and the struct
has a `preview: Option<String>` field, but `detect_clones` never calls it. Every fragment
in JSON output has `preview` absent.

**v2 Fix:** After determining fragment line range, read those lines from the source file
and populate `preview` via `with_preview(source_lines)`.

### BUG-6: Fixed 25-Token Windows Instead of Syntactic Boundaries

**Location:** `extract_fragments` lines 1615-1640  
**Problem:** The sliding window uses `window_size = min_tokens` (default 25 tokens) with
step `window_size / 2`. This is purely positional — windows cut through function
boundaries, expressions, and statement blocks. Two semantically identical functions split
across window boundaries will be missed. Two unrelated windows that happen to share
boundary tokens will be falsely matched.

**v2 Fix:** Use tree-sitter to extract syntactic units (functions, methods, classes) as
fragments instead of fixed-size windows. Fall back to fixed windows only for languages/
files where function-level extraction fails.

---

## 8. Preserved Behaviors (v2 Must Keep)

### Type Hierarchy (identical field names, order, visibility)

v2 MUST export the same public types with identical field names and serde attributes:

- `ClonesReport` — same fields, same serde rules
- `ClonePair` — same fields, same canonical() method
- `CloneFragment` — same fields, same with_preview()/with_function() builders
- `CloneType` — same variants, same serde renames ("Type-1", "Type-2", "Type-3")
- `CloneClass` — same fields
- `CloneStats` — same fields
- `CloneConfig` — same fields
- `NormalizationMode` — same variants, same rename_all="lowercase"
- `ClonesOptions` — same fields (all 11)

### JSON Serialization Format (byte-compatible)

- All `skip_serializing_if` rules preserved exactly
- All serde renames preserved exactly
- `class_count: None` absent from JSON (not null)
- `clone_classes: []` absent from JSON when empty
- `interpretation` absent when None

### Filter Functions

Exact same logic for `is_test_file` and `is_generated_file` — do not add/remove patterns
without updating tests.

### File Discovery

- walkdir traversal (no symlink following by default)
- `max_files` hard cap via early break
- Errors in directory entries silently skipped

### Union-Find for Clone Classes

Same algorithm: path-compressed union by rank. `compute_clone_classes` takes `&[ClonePair]`
and returns `Vec<CloneClass>`. Classes with < 2 members excluded.

### Similarity Classification Thresholds

- Type-1: `|sim - 1.0| < 1e-9`
- Type-2: `sim >= 0.9 - 1e-9`
- Type-3: `sim < 0.9`

These must not change — downstream consumers depend on these boundaries.

### RollingHash Parameters

- Base: `31`
- Modulus: `1_000_000_007`
- Wrapping arithmetic for overflow safety

---

## 9. v2 Requirements (New Behavior)

### REQ-1: Function-Level Fragment Extraction

Use tree-sitter to identify top-level syntactic units per language:

| Language | Fragment units |
|---|---|
| Python | `function_definition`, `class_definition` |
| TypeScript/JS | `function_declaration`, `method_definition`, `arrow_function` (when assigned) |
| Go | `function_declaration`, `method_declaration` |
| Rust | `function_item`, `impl_item` |
| Java | `method_declaration`, `class_declaration` |

Fall back to fixed-window sliding if AST extraction yields < 2 fragments for a file.

Fragment granularity rule: if a syntactic unit has < `min_tokens` tokens, merge with
adjacent sibling until threshold is met. If the whole file is < `min_tokens`, skip.

### REQ-2: Karp-Rabin on Raw Token Sequences for Type-1/2

For Type-1 detection: hash raw tokens (no normalization). Two fragments with identical
raw token sequences get the same hash → Type-1.

For Type-2 detection: hash normalized tokens ($ID/$STR/$NUM substituted). Two fragments
with identical normalized sequences get the same hash → candidate Type-2. Then verify
by comparing original tokens — if 100% similar, reclassify as Type-1.

This means:
- Type-1 hash index built from raw tokens
- Type-2 hash index built from normalized tokens
- Both hashes computed per fragment
- Two hash passes: first match raw hashes (Type-1), then match normalized hashes (Type-2)

### REQ-3: Real Line Numbers from Tree-Sitter

When creating `TokenSequence` and later `CloneFragment`:

```rust
let start_line = node.start_position().row + 1; // row is 0-indexed
let end_line   = node.end_position().row   + 1;
```

These must be read from the tree-sitter `Node` corresponding to the syntactic boundary,
not computed from token indices.

### REQ-4: Populate preview Field

After computing `start_line` and `end_line`, extract the corresponding source lines:

```rust
let source_lines: Vec<&str> = source.lines()
    .skip(start_line - 1)
    .take(end_line - start_line + 1)
    .collect();
let preview_text = source_lines.join("\n");
fragment = fragment.with_preview(preview_text); // truncated to 100 chars internally
```

### REQ-5: Correct include_within_file Semantics

```rust
// v2: skip ALL same-file pairs when include_within_file = false
if !options.include_within_file && file_idx1 == file_idx2 {
    continue;  // unconditional skip, no ranges_overlap check
}
```

Same-file overlapping pairs must also be skipped regardless of this flag (they are always
invalid).

### REQ-6: Enforce min_lines

```rust
fn extract_fragments(..., min_lines: usize) -> Vec<TokenSequence> {
    // After computing start_line / end_line from tree-sitter:
    let line_count = end_line - start_line + 1;
    if line_count < min_lines {
        continue; // skip fragment
    }
    // ...
}
```

### REQ-7: Normalization Strategy for Type-2

Normalization should only be applied for Type-2 candidate hashing, not for the primary
Dice similarity comparison:

1. Compute raw hash → if match → verify raw Dice → if >= 0.99 → Type-1
2. Compute normalized hash → if match → verify normalized Dice → if >= 0.9 → Type-2
3. For Type-3: inverted index on raw token values, Dice on raw tokens

This prevents the catastrophic false positive issue where normalizing everything to $ID
makes unrelated code appear similar.

---

## 10. Architecture Map

```
CLI (ClonesArgs::run)
    │
    ▼
detect_clones(path, options)
    │
    ├─→ discover_source_files()          [walkdir + extension filter]
    │       ├─ is_test_file()
    │       ├─ is_generated_file()
    │       └─ is_source_file_for_clones()
    │
    ├─→ tokenize_files()                 [per-file pipeline]
    │       ├─ tokenize_file()           [parse_file → has_parse_errors → extract_tokens_from_ast]
    │       │       └─ extract_tokens_recursive()   [AST walk]
    │       │               ├─ is_comment_node()
    │       │               ├─ should_capture_as_token()
    │       │               └─ categorize_token()
    │       ├─ apply_normalization()
    │       │       └─ normalize_single_token()
    │       └─ extract_fragments()       [BUG: fixed windows, fabricated line numbers]
    │               └─ compute_fragment_hash()
    │                       └─ RollingHash
    │
    ├─→ [hash index loop]                [Type-1/2 detection]
    │       ├─ compute_dice_similarity() [verify]
    │       ├─ classify_clone_type()
    │       ├─ CloneFragment::new()
    │       └─ ClonePair::new().canonical()
    │
    ├─→ [Type-3 phase]
    │       ├─ build_inverted_index()
    │       ├─ find_type3_candidates()
    │       └─ compute_dice_similarity() [verify]
    │
    └─→ compute_clone_classes()          [if show_classes]
            └─ UnionFind
```

---

## 11. Key File Locations

| File | Purpose |
|---|---|
| `crates/tldr-core/src/analysis/clones.rs` | Core implementation (2201 lines) |
| `crates/tldr-cli/src/commands/clones.rs` | CLI command handler (157 lines) |
| `crates/tldr-core/src/analysis/clones_tests.rs` | Unit tests |
| `crates/tldr-core/src/analysis/clones_v2/` | v2 implementation target |
| `crates/tldr-core/src/ast/parser.rs` | `parse()` and `parse_file()` used for tokenization |
| `crates/tldr-core/src/types.rs` | `Language` enum used in AST parsing |


---

## Adversarial Review — Pass 1: Failure Modes

Reviewer: architect-agent (Opus 4.6)  
Date: 2026-02-18  
Scope: Failure modes, edge cases, performance cliffs, and correctness risks in the v2 spec and v1 implementation.

---

### RISK-1: O(n^2) Blowup in Type-3 Inverted Index with Normalized Tokens

- **Severity:** CRITICAL
- **Scenario:** When normalization mode is `All` (the default), all identifiers become `$ID`, all strings become `$STR`, all numbers become `$NUM`. The inverted index maps *unique token values* to fragment lists. With normalization, `$ID` appears in nearly every fragment. A single inverted index lookup for `$ID` returns ALL fragments, producing O(n^2) candidate pairs.
- **Impact:** The spec (REQ-7) says Type-3 should use "raw token values" for the inverted index, which mitigates this. But the v1 code at line 1282 uses `frag.tokens` which are *already normalized* (normalization happens in `tokenize_files` at line 1575, *before* fragment extraction). If v2 does not carefully separate raw vs. normalized token storage, the same collapse occurs. With 1000 fragments, the `$ID` posting list contains all 1000 entries, yielding ~500K candidate pairs. With 5000 fragments (a mid-size monorepo), that is 12.5M pairs.
- **Mitigation:**
  1. Store both raw and normalized tokens per fragment (REQ-7 partially addresses this, but does not specify the data structure).
  2. In `TokenSequence`, add a `raw_tokens: Vec<NormalizedToken>` field alongside the normalized tokens.
  3. Build the inverted index exclusively from `raw_tokens` values.
  4. Add a runtime guard: if any posting list exceeds `MAX_POSTING_LIST_SIZE` (e.g., 500), skip that token for indexing (it has no discriminative power). This is the SourcererCC "sub-block overlap filtering" insight.

### RISK-2: Memory Explosion on Large Files (10K+ Lines, 100K+ Tokens)

- **Severity:** HIGH
- **Scenario:** A single 10,000-line file with ~80,000 tokens. In v1, the sliding window approach with `window_size=25` and `step=12` produces `(80000 - 25) / 12 ≈ 6,664` fragments *per file*. Each fragment stores a `Vec<NormalizedToken>` of 25 tokens. That is 6,664 * 25 = 166,600 token objects, each with two `String` fields (`value` + `original`). For v2 with function-level extraction, this is less severe—but the fallback to sliding windows (REQ-1: "Fall back to fixed-window sliding if AST extraction yields < 2 fragments") re-introduces the problem for files without clear function boundaries (e.g., scripts, configuration-as-code, Jupyter notebook cells).
- **Impact:** With 100 such files, the `all_fragments` vector holds 666,400 entries. Each `TokenSequence` contains a full `Vec<NormalizedToken>` clone. At ~200 bytes per token (two strings + category enum), that is ~3.3 GB for the token data alone. The hash index and inverted index add more.
- **Mitigation:**
  1. For the sliding-window fallback, cap fragment count per file: `MAX_FRAGMENTS_PER_FILE = 200`. If a file would produce more, increase the step size to stay under the cap.
  2. Consider using token indices into a shared per-file token array instead of cloning token vectors into each fragment. Fragments would store `(file_idx, token_start, token_end)` and borrow from the file's token array.
  3. Add a `max_file_tokens` option (default: 50,000). Files exceeding this are skipped with a warning. PMD CPD has `maxSize: 100KB` for this exact reason.

### RISK-3: Very Small Files Produce Zero Fragments and Silent Data Loss

- **Severity:** MEDIUM
- **Scenario:** A utility file with a single 20-token function. With `min_tokens=25` (v1 default) or `min_tokens=50` (prior art recommendation), the file is silently excluded because `tokens.len() < min_tokens` in `extract_fragments`. For v2 with function-level extraction, a single function with 20 tokens still fails the `min_tokens` check.
- **Impact:** Users analyzing small utility libraries (e.g., a collection of 15-token helper functions) get zero results with no indication that files were excluded. They may believe there are no clones when in fact the thresholds excluded all candidates.
- **Mitigation:**
  1. Track and report `files_excluded_below_threshold` in `CloneStats`. Add a new field: `pub skipped_files: usize`.
  2. In text output mode, print a warning: "N files skipped (below min_tokens threshold)".
  3. Document in CLI help that `--min-tokens 25` excludes functions with fewer than 25 tokens.
  4. Consider the REQ-1 merge strategy: "if a syntactic unit has < min_tokens tokens, merge with adjacent sibling until threshold is met." This is specified but needs careful implementation—merging two unrelated small functions inflates false positives.

### RISK-4: Decorators and Annotations Inflate Function Token Counts and Corrupt Line Ranges

- **Severity:** HIGH
- **Scenario:** A Python function with 5 decorators:
  ```python
  @app.route("/api/v1/users", methods=["GET"])
  @requires_auth
  @cache(timeout=300)
  @rate_limit(100)
  @validate_params(schema=UserSchema)
  def get_users():
      return db.query(User).all()
  ```
  Tree-sitter's `function_definition` node in Python includes decorators as child nodes (`decorator` nodes precede the `def` keyword). The `start_line` of the `function_definition` node is line 1 (the first decorator), not line 6 (the `def`). The function body is only 2 lines, but the fragment reports 7 lines. Two functions with identical bodies but different decorators will have different token sequences—the decorators add ~30 tokens of noise.
- **Impact:** (a) Line ranges are misleading—the reported fragment includes decorator lines that are not part of the function logic. (b) Type-1/2 detection fails because decorator tokens differ even when function bodies are identical. (c) Token counts are inflated, causing small functions to clear `min_tokens` thresholds due to decorator padding.
- **Mitigation:**
  1. The prior art synthesis (Decision 6) already recommends "skip annotation/attribute nodes." Implement this by walking the `function_definition` node's children and skipping `decorator` nodes during token extraction.
  2. For line ranges, use the `def` keyword node's `start_position()` instead of the `function_definition` node's `start_position()`. In tree-sitter Python, the `function_definition` has a `name` child—use `node.child_by_field_name("name")` to find the function name node and get its row.
  3. Similarly for Java `@Override`, `@Autowired`, etc.: skip `annotation` and `marker_annotation` nodes. For Rust: skip `attribute_item` nodes inside `function_item`.

### RISK-5: Nested Functions and Closures Produce Duplicate or Subsumption Artifacts

- **Severity:** HIGH
- **Scenario:** Python closures:
  ```python
  def outer():
      def inner():
          do_work()
      return inner
  ```
  Tree-sitter query `(function_definition) @fn` matches BOTH `outer` and `inner`. The token sequence for `outer` includes all tokens of `inner`. If `inner` is also extracted as a separate fragment, then:
  - `outer` vs. some other function may match because of `inner`'s tokens (subsumption noise).
  - `inner` is reported twice: once standalone, once as part of `outer`.
  
  Same issue with Rust closures inside `impl` blocks, JavaScript arrow functions inside functions, and Go anonymous functions.
- **Impact:** Duplicate clone pairs (outer-as-a-whole matches another outer, AND inner matches another inner). Inflated clone counts. Misleading similarity scores because the outer function's bag includes inner function tokens.
- **Mitigation:**
  1. Extract only *top-level* syntactic units by default. Use tree-sitter query patterns that exclude nested matches. For example, in Python: `(function_definition body: (block (function_definition) @nested))` to identify nested functions, then exclude `@nested` from the main extraction.
  2. Alternatively, extract all matches but apply **subsumption filtering** (already recommended in prior art Decision 6): if fragment A fully contains fragment B (A.start_line <= B.start_line AND A.end_line >= B.end_line AND same file), prefer the smaller fragment B and exclude A from bag comparison. Or: for each detected clone pair, check if one fragment is a strict sub-range of another fragment in the same file, and suppress the larger one.
  3. For the REQ-1 tree-sitter queries, add a note that `(function_item) @fn` in Rust will also match closures assigned to variables. Filter by node depth or parent node kind.

### RISK-6: Generated Code That Looks Like Real Code (Not Caught by is_generated_file)

- **Severity:** MEDIUM
- **Scenario:** `is_generated_file` checks file paths for patterns like `_generated.go`, `.pb.go`, `vendor/`, etc. But many code generators produce files with normal names:
  - `sqlc` generates `query.sql.go` or `models.go` in the same directory as hand-written code.
  - `swagger-codegen` generates `api_client.go`, `model_user.go` with standard names.
  - `prisma` generates `@prisma/client/index.d.ts` inside `node_modules/` (caught), but also runtime files outside `node_modules/`.
  - `openapi-generator` outputs to a configurable directory with normal file names.
  
  These files are highly repetitive (CRUD boilerplate) and will dominate clone detection results.
- **Impact:** Clone results overwhelmed by generated code that the user does not control. Every generated model file matches every other generated model file, filling the `max_clones` limit.
- **Mitigation:**
  1. Check for common generation markers in file content (first 5 lines):
     - `// Code generated by` (Go convention, also protobuf, gRPC)
     - `# Generated by` (Python)
     - `/* AUTO-GENERATED */`
     - `// DO NOT EDIT`
     - `@generated` (Meta/Facebook convention)
  2. Add `content_check_generated: bool` option (default: true when `exclude_generated` is true). Read the first 512 bytes of each file and check for generation markers.
  3. This is what PMD CPD does with suppression comments (`CPD-OFF`), and what many linters do (eslint checks for `/* eslint-disable */`).

### RISK-7: Tree-Sitter Parser Version Lag for New Language Features

- **Severity:** MEDIUM
- **Scenario:** Python 3.10+ `match` statements, Python 3.12 f-string improvements, TypeScript 5.x `satisfies` operator, Rust `let-else` patterns. If the tree-sitter grammar bundled in `tldr-rs` is outdated, these constructs produce `ERROR` nodes in the AST.
- **Impact:** The `has_parse_errors` function tolerates up to 50% error nodes. A file using modern syntax throughout may have >50% errors and be silently skipped. A file with a few modern constructs may have <50% errors but produce garbled token sequences (ERROR nodes are skipped, losing tokens that should be part of the fragment).
- **Mitigation:**
  1. Document the tree-sitter grammar versions bundled with each release.
  2. Add a `--warn-parse-errors` flag that reports files with any ERROR nodes (not just >50%).
  3. For partial parse errors (<50%), log a warning with the file path and error count. Currently this is completely silent.
  4. Consider lowering the error threshold or making it configurable: `--max-parse-error-ratio 0.1` (default: 0.5).
  5. Pin and regularly update tree-sitter grammars. Add a CI check that parses a corpus of modern syntax files and verifies zero ERROR nodes.

### RISK-8: Hash Collision Rate with Rabin-Karp (base=31, modulus=10^9+7)

- **Severity:** LOW (mitigated by Dice verification)
- **Scenario:** The rolling hash uses base=31 and modulus=10^9+7, producing hash values in range [0, 10^9+6]. For Type-1/2 detection, two fragments with different token sequences may hash to the same value. With N fragments, the birthday paradox gives collision probability ~ N^2 / (2 * 10^9). For 10,000 fragments: 10^8 / 2*10^9 = 5% chance of at least one collision. For 50,000 fragments: ~100% probability of collisions.
- **Impact:** Hash collisions are caught by `compute_dice_similarity` verification (line 1122), so they do not produce false positives. However, each collision requires a full Dice comparison, which is O(|tokens1| + |tokens2|). If collisions are frequent, this degrades performance.
- **Mitigation:**
  1. The current approach is already correct (hash + verify). The collision rate is acceptable for codebases under 10K fragments.
  2. For larger codebases, consider double-hashing: compute two independent rolling hashes with different (base, modulus) pairs. Only bucket together fragments where BOTH hashes match. This reduces collision probability to ~N^2 / (2 * 10^18), which is negligible.
  3. Alternatively, switch to xxHash or FNV-1a for the fragment-level hash (not rolling, but applied to the full token sequence). These produce 64-bit hashes with better distribution. The prior art synthesis recommends xxHash for Phase 1.

### RISK-9: Unicode Identifiers Break Token Hashing and Comparison

- **Severity:** MEDIUM
- **Scenario:** Python 3 allows Unicode identifiers: `def berechne_flache(lange, breite):` or even `def 計算面積(長さ, 幅):`. Rust allows them too. JavaScript/TypeScript allow Unicode in identifiers. Tree-sitter correctly parses these as `identifier` nodes with the full Unicode text.
- **Impact:** (a) The `hash_token` function (line 1845) uses `token.value.chars()` with `c as u64`, which correctly handles Unicode code points. However, two visually identical Unicode strings using different normalization forms (NFC vs NFD) will produce different hashes. The character `e` (U+00E9) vs. `e` + combining accent (U+0065 U+0301) are visually identical but hash differently. (b) Under normalization mode `All`, identifiers become `$ID` so this is masked. But under `None` or `Literals` mode, Unicode normalization differences cause false negatives. (c) Homoglyph attacks: Cyrillic `a` (U+0430) vs Latin `a` (U+0061) in identifiers would not be detected as clones.
- **Mitigation:**
  1. Apply Unicode NFC normalization to all token values during extraction, before hashing. Rust's `unicode-normalization` crate provides this.
  2. This is a low-frequency issue in practice—most codebases use ASCII identifiers. But it should be documented as a known limitation if not addressed.
  3. For the `None` normalization mode, add a note that Unicode-equivalent identifiers may not be detected as clones.

### RISK-10: Go Methods vs Free Functions Have Different AST Shapes

- **Severity:** MEDIUM
- **Scenario:** Go has two function patterns:
  ```go
  // Free function
  func ProcessUser(u User) error { ... }
  
  // Method (receiver function)  
  func (s *Service) ProcessUser(u User) error { ... }
  ```
  Tree-sitter Go grammar uses `function_declaration` for free functions and `method_declaration` for methods. The spec REQ-1 correctly lists both. However, the token sequences differ: the method has receiver tokens `(`, `s`, `*`, `Service`, `)` prepended. Two identical function bodies—one a free function, one a method—will NOT be Type-1 clones because the receiver tokens differ.
- **Impact:** Refactoring a free function to a method (or vice versa) will not be detected as a Type-1 clone. This is technically correct behavior (they ARE different), but users may expect them to match as Type-2 clones since only the receiver changed.
- **Mitigation:**
  1. During token extraction for Go functions, optionally strip the receiver parameter list. This could be controlled by a `--ignore-receivers` flag.
  2. Alternatively, treat receiver parameters as identifiers that get normalized to `$ID` under normalization mode `All`. This is already the case, but the `*` and type name add structural tokens that differ.
  3. Document this as expected behavior: "Go receiver parameters are included in fragment tokens. Two functions differing only in receiver type will be detected as Type-2 or Type-3 clones, not Type-1."

### RISK-11: Rust Trait Impl Blocks Produce Overlapping Fragments

- **Severity:** MEDIUM
- **Scenario:** The spec REQ-1 lists `impl_item` as a Rust fragment unit alongside `function_item`. An `impl` block contains multiple `function_item` children:
  ```rust
  impl Display for MyType {
      fn fmt(&self, f: &mut Formatter) -> Result { ... }
  }
  ```
  Tree-sitter query `(function_item) @fn` matches `fmt`. But `(impl_item) @impl` matches the entire impl block. If both queries are used, `fmt` is extracted twice: once as a standalone function fragment, and once as part of the impl block fragment.
- **Impact:** Same subsumption problem as RISK-5. The impl block's tokens include all method tokens, leading to inflated similarity scores and duplicate pairs.
- **Mitigation:**
  1. Choose one granularity: extract `function_item` nodes inside `impl_item` as individual fragments, and do NOT extract the `impl_item` as a whole fragment. The impl block is a container, not a semantic unit for clone detection.
  2. Revise REQ-1 for Rust: replace `impl_item` with `(impl_item (function_item) @method)` to extract methods within impl blocks. Keep standalone `(function_item) @fn` for free functions.
  3. Same principle applies to Java `class_declaration`: do not extract the class as a fragment; extract its `method_declaration` children.

### RISK-12: The "Merge Adjacent Siblings" Strategy (REQ-1) Is Underspecified

- **Severity:** HIGH
- **Scenario:** REQ-1 states: "if a syntactic unit has < min_tokens tokens, merge with adjacent sibling until threshold is met." Consider a file with 10 small functions, each with 15 tokens:
  ```
  fn a() { ... }  // 15 tokens
  fn b() { ... }  // 15 tokens
  fn c() { ... }  // 15 tokens
  ...
  ```
  With `min_tokens=50`, we need to merge at least 4 adjacent functions to reach the threshold. But these functions may be completely unrelated. Merging `a+b+c+d` into one fragment and comparing it against another merged fragment `e+f+g+h` is semantically meaningless.
- **Impact:** False positives from merged unrelated functions. The merged fragment has a token bag that is an arbitrary mix of unrelated code. Two files with similarly-sized small functions will produce high Dice similarity on their merged fragments because the structural tokens (braces, keywords) dominate.
- **Mitigation:**
  1. Do NOT merge unrelated functions. Instead, if a function has < `min_tokens` tokens, simply exclude it. Small functions are unlikely to be meaningful clones.
  2. Only merge functions that are structurally related: e.g., methods within the same class/impl block.
  3. Alternatively, reduce `min_tokens` for function-level extraction (e.g., `min_tokens=15` for functions, `min_tokens=50` for file-level windows). The spec should distinguish between function-level and window-level minimum thresholds.
  4. Add a separate `min_function_tokens` option (default: 15) distinct from `min_tokens` (which controls window-level extraction).

### RISK-13: Preview Truncation at 100 Characters Loses Context

- **Severity:** LOW
- **Scenario:** The `with_preview` method truncates to 100 characters with `...`. For a function like:
  ```python
  def calculate_monthly_subscription_price_with_discounts_and_taxes(base_price, discount_pct, tax_rate):
  ```
  The first line alone is 100+ characters. The preview becomes the function signature truncated mid-parameter, with no body visible.
- **Impact:** Preview is useless for understanding what the clone is. Users must open the file to see the actual code.
- **Mitigation:**
  1. Increase default preview limit to 500 characters or 10 lines (whichever is shorter).
  2. Make the preview limit configurable: `--preview-chars 500`.
  3. Consider showing the first N lines instead of the first N characters, since line breaks are more natural boundaries.

### RISK-14: found_pairs Deduplication Uses Fabricated Line Numbers as Keys

- **Severity:** HIGH (in v1; addressed by v2 if line numbers are correct)
- **Scenario:** In v1, `create_pair_key` uses `(file, start_line)` as the deduplication key. But `start_line` is fabricated (BUG-1). Two different sliding windows that happen to produce the same fabricated `start_line` (because `start / 10 + 1` collides for different `start` values) would be deduplicated incorrectly, causing real clone pairs to be silently dropped.
- **Impact:** In v1: missed clones due to key collision on fabricated line numbers. In v2: this should be fixed by REQ-3 (real line numbers), BUT there is a subtlety—two different function fragments in the same file with the same start line (e.g., a decorator on line 10 and the function def on line 10) could still collide in the key.
- **Mitigation:**
  1. Use `(file, start_line, end_line)` as the deduplication key instead of just `(file, start_line)`. This makes collisions far less likely.
  2. Or use `(file, start_line, token_count)` for even stronger dedup.
  3. Verify in v2 that `create_pair_key` is updated to use the corrected line numbers.

### RISK-15: Type-3 Phase Silently Skips Pairs with Similarity >= 0.9

- **Severity:** MEDIUM
- **Scenario:** In v1 line 1211: `if similarity < options.threshold || similarity >= 0.9 { continue; }`. This means the Type-3 phase skips any pair with similarity >= 0.9, assuming it was already found by the Type-1/2 hash phase. But hash-based detection only finds pairs with IDENTICAL hashes. Two fragments with 95% Dice similarity but different hashes (which is common—a single token difference changes the hash) would be skipped by BOTH phases: the hash phase misses them (different hashes), and the Type-3 phase filters them out (>= 0.9).
- **Impact:** Clone pairs with similarity in [0.9, 1.0) that happen to have different fragment hashes are never reported. These are Type-2 clones that fall through the cracks.
- **Mitigation:**
  1. In v2, the two-hash strategy (REQ-2: raw hash for Type-1, normalized hash for Type-2) should catch these. If two fragments have 95% similarity on raw tokens, they likely have identical normalized hashes and will be caught in the Type-2 hash phase.
  2. However, if they differ in structural tokens (not just identifiers/literals), the normalized hashes will also differ. Remove the `similarity >= 0.9` filter from the Type-3 phase. Instead, check against `found_pairs` to avoid duplicates (which the code already does at line 1198).
  3. The Type-3 phase should report all pairs above `threshold` that were not already found by Type-1/2 hash matching, regardless of similarity level.

### RISK-16: Race Condition Potential if Parallelized

- **Severity:** LOW (v1 is single-threaded, but relevant for v2)
- **Scenario:** The spec does not mention parallelism, but tokenization of files is embarrassingly parallel and likely to be parallelized in v2 for performance. The `tokenize_files` function iterates sequentially over files. If parallelized with `rayon::par_iter`, the shared mutable state (`total_tokens`, `file_fragments`) would need synchronization.
- **Impact:** Data races if parallelization is done naively.
- **Mitigation:**
  1. Use `rayon::par_iter().map()` to produce per-file results, then collect into a vector. Do not use shared mutable state.
  2. Ensure `file_fragments` ordering is deterministic (same order as input files) so that fragment indices remain stable for the hash index phase.
  3. Document that the detection phase (hash matching, inverted index) must remain single-threaded because `found_pairs` and `clone_pairs` are shared mutable state.

### RISK-17: NormalizationMode::None Eliminates Type-2 Detection Entirely

- **Severity:** MEDIUM
- **Scenario:** With `normalization = None`, no identifiers or literals are replaced. The hash index only groups fragments with byte-identical token sequences. The Dice similarity is computed on raw token values. Two functions identical except for variable names (textbook Type-2 clone) will have different hashes AND low raw Dice similarity (each identifier is a unique string, not `$ID`).
- **Impact:** `--normalize none` effectively disables Type-2 detection. Users who set this expecting "less aggressive" detection actually get no Type-2 results at all.
- **Mitigation:**
  1. Document clearly: "`--normalize none` detects only Type-1 (exact) clones. For Type-2 detection, use `--normalize identifiers` or `--normalize all`."
  2. In v2 with the two-hash strategy (REQ-2), Type-2 detection uses a normalized hash regardless of the user's normalization setting. This means `NormalizationMode` should control only the Dice similarity comparison display, not the internal Type-2 hash computation. Clarify this in the spec.
  3. If the user explicitly requests `--normalize none --type-filter 2`, warn that this combination will produce no results.

### RISK-18: CloneFragment Equality Based on All Fields Including Optional Ones

- **Severity:** LOW
- **Scenario:** `CloneFragment` derives `PartialEq, Eq, Hash` and is used as a HashMap key in `compute_clone_classes` (line 1681). It includes `function: Option<String>` and `preview: Option<String>` in the derived comparison. In v2, `preview` will be populated (REQ-4). If two references to the same code location have slightly different previews (e.g., due to trailing whitespace differences in file reads), they will be treated as different fragments in the Union-Find.
- **Impact:** Clone classes may fail to merge fragments that refer to the same code location, producing fragmented/incomplete clone classes.
- **Mitigation:**
  1. Implement manual `PartialEq`, `Eq`, and `Hash` for `CloneFragment` that only compare `(file, start_line, end_line)`, ignoring `preview`, `function`, `tokens`, and `lines`.
  2. Or use a separate `FragmentKey` struct for HashMap keying that only includes the location fields.

---

### Summary Table

| Risk | Severity | Category | Addressed by Spec? |
|------|----------|----------|---------------------|
| RISK-1: Normalized inverted index O(n^2) | CRITICAL | Performance | Partially (REQ-7 mentions raw tokens, no posting list cap) |
| RISK-2: Memory explosion on large files | HIGH | Performance | No |
| RISK-3: Small files silently excluded | MEDIUM | UX/Correctness | No |
| RISK-4: Decorators inflate token counts | HIGH | Correctness | Partially (prior art mentions, not in REQ) |
| RISK-5: Nested functions produce subsumption | HIGH | Correctness | No |
| RISK-6: Generated code with normal names | MEDIUM | Correctness | No (only path-based detection) |
| RISK-7: Tree-sitter grammar version lag | MEDIUM | Correctness | No |
| RISK-8: Hash collision rate | LOW | Performance | Mitigated (Dice verification) |
| RISK-9: Unicode identifier normalization | MEDIUM | Correctness | No |
| RISK-10: Go methods vs free functions | MEDIUM | Correctness | Partially (both queries listed, behavior unspecified) |
| RISK-11: Rust impl blocks produce overlap | MEDIUM | Correctness | No (spec lists impl_item as fragment unit) |
| RISK-12: Merge adjacent siblings underspecified | HIGH | Correctness | Mentioned but underspecified |
| RISK-13: Preview truncation too aggressive | LOW | UX | No |
| RISK-14: Dedup key uses only start_line | HIGH | Correctness | Indirectly (REQ-3 fixes line numbers) |
| RISK-15: Type-3 skips similarity >= 0.9 | MEDIUM | Correctness | Partially (REQ-2 two-hash helps) |
| RISK-16: Race condition if parallelized | LOW | Correctness | Not applicable yet |
| RISK-17: normalize=none kills Type-2 | MEDIUM | UX/Correctness | Partially (REQ-7 unclear) |
| RISK-18: Fragment equality includes preview | LOW | Correctness | No |

**Critical path for v2:** RISK-1, RISK-4, RISK-5, RISK-11, RISK-12, and RISK-15 must be addressed in the implementation spec before coding begins. RISK-2 should have a mitigation plan (even if deferred) to avoid shipping a tool that OOMs on real monorepos.
