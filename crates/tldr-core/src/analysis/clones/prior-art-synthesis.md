# Prior Art Synthesis: Code Clone Detection

Generated: 2026-02-18
Research Agent: Oracle (Opus 4.6)

## Summary

This document synthesizes implementation patterns from seven major code clone detection
tools and Rust ecosystem options. The goal is to inform concrete design decisions for
tldr-rs v2's `clones_v2` module, which uses tree-sitter for parsing and Rust for
implementation. Each decision area includes a comparison matrix and a recommended
approach with rationale.

---

## Tool Profiles

### 1. rust-code-analysis (Mozilla)

**Clone Detection?** No. rust-code-analysis is a metrics extraction library (cyclomatic
complexity, Halstead, SLOC, cognitive complexity, etc.) built on tree-sitter. It does
NOT include clone detection. Its related research paper CLEVER (2025, Journal of Systems
and Software) combines code metrics WITH clone detection for fault prevention, but the
clone detection is external -- not part of rust-code-analysis itself.

**What we can adopt:**
- Tree-sitter AST traversal patterns in Rust (it is the canonical example)
- Language-agnostic metric extraction architecture
- Node visitor patterns for extracting function/block boundaries

**Source:** [GitHub](https://github.com/mozilla/rust-code-analysis) |
[Paper](https://www.sciencedirect.com/science/article/pii/S2352711020303484)

---

### 2. jscpd (kucherenko)

**Algorithm:** Rabin-Karp rolling hash on token sequences.
**Languages:** 150+ via Prism.js tokenizer.
**Clone Types:** Type-1 and Type-2 only (exact and renamed). Cannot detect Type-3.

**Fragment Extraction:** Sliding window over token stream. No AST awareness -- purely
lexical. Fragments are defined by the window size (`minTokens`). The tokenizer breaks
source into tokens using Prism.js (language-aware syntax highlighting tokenizer).

**Matching Algorithm:** Rabin-Karp rolling hash. Tokens are hashed into fingerprints,
and fingerprint equality triggers character-by-character verification to eliminate hash
collisions.

**Normalization:** Three modes:
- `strict` -- all token types preserved
- `mild` -- skip newlines and empty symbols
- `weak` -- skip newlines, empty symbols, and comments

No identifier or literal normalization (blind renaming). This means Type-2 detection
relies on the hash matching tokens structurally rather than by value.

**Defaults:**
- `minLines`: 5
- `minTokens`: 50
- `maxLines`: 1000
- `maxSize`: 100kb per file

**Line Number Tracking:** Token-to-source mapping maintained by the Prism.js tokenizer.
Each token retains its source position.

**False Positive Mitigation:**
- Min tokens/lines thresholds filter trivially small matches
- Max file size limits prevent pathological cases
- Detection mode (`strict`/`mild`/`weak`) controls noise

**Source:** [GitHub](https://github.com/kucherenko/jscpd) |
[npm](https://www.npmjs.com/package/jscpd)

---

### 3. PMD CPD

**Algorithm:** Karp-Rabin string matching on token sequences (third iteration; previously
used Greedy String Tiling, then Burrows-Wheeler transform).
**Languages:** 33+ via language-specific lexers (JavaCC and ANTLR grammars).
**Clone Types:** Type-1 and Type-2.

**Fragment Extraction:** Linear token stream. CPD tokenizes the entire file into a
flat token sequence. No function/block boundaries -- matching occurs across the
entire token stream. Matches are contiguous token subsequences.

**Matching Algorithm:** Karp-Rabin. The `MatchAlgorithm` class computes rolling hashes
over token subsequences of length `--minimum-tokens`. When hashes match, exact token
comparison confirms the clone. All matches are exact (100% similarity after normalization).

**Normalization (Identity Abstraction):**
- `--ignore-identifiers`: Replace ALL identifiers with a uniform placeholder BEFORE
  matching. This is blind renaming (not consistent/parameterized).
- `--ignore-literals`: Replace ALL literals with a uniform placeholder.
- `--ignore-literal-sequences`: Ignore literal-only sequences.
- `--ignore-annotations`: Skip annotation tokens (Java/Kotlin).
- These are applied during tokenization, before the match algorithm runs.
- Default: ALL disabled (exact matching only).

**Defaults:**
- `--minimum-tokens`: 100 (recommended starting point; no hard default in CLI)
- Maven plugin commonly uses 100
- Recommendations: 50 for new code, 100 for legacy code

**Line Number Tracking:** Each token in the sequence carries its source file and line
number. The `Match` object stores begin/end token indices which map back to source
locations.

**False Positive Mitigation:**
- `CPD-OFF` / `CPD-ON` suppression comments in source
- Exclusion files listing classes exempt from checking
- Annotation ignoring (Java frameworks produce repetitive annotations)
- `--skip-lexical-errors` to handle unparseable files
- `NOPMD` comment markers

**Source:** [PMD Docs](https://docs.pmd-code.org/pmd-doc-6.55.0/pmd_userdocs_cpd.html) |
[GitHub](https://github.com/pmd/pmd)

---

### 4. SourcererCC

**Algorithm:** Bag-of-tokens overlap similarity with partial inverted index.
**Languages:** Java, C, C++, C#, Python (extensible via parsers).
**Clone Types:** Type-1, Type-2, Type-3.

**Fragment Extraction:** Language-aware parser extracts fragments at configurable
granularity: **file-level**, **method/function-level**, or **block-level** (statements
between `{}`). The parser is decoupled from the detection engine -- you can plug in
any parser that produces `(file, startLine, endLine, tokenBag)` tuples.

**Matching Algorithm:** NOT sequence-based. Uses **bag-of-tokens** (multiset) overlap:
```
overlap_similarity(A, B) = |A ∩ B| / max(|A|, |B|)
```
Two optimizations make this scalable to 250 MLOC:
1. **Sub-block Overlap Filtering**: Only 30% of each block's tokens are indexed
   (for 70% threshold). The inverted index maps tokens to code blocks.
2. **Token Position Filtering**: Exploits token ordering within a block to compute
   live upper/lower bounds on similarity. Candidates eliminated early if upper bound
   falls below threshold; accepted early if lower bound exceeds threshold.

**Normalization:** Token-level. Identifiers and literals can be abstracted, but the
bag-of-tokens approach inherently handles renaming because it's order-independent.

**Default Threshold:** 70% overlap similarity (optimum precision/recall per paper).
Configurable via `runnodes.sh` (8 = 80%, 7 = 70%, etc.).

**Line Number Tracking:** Each extracted block stores `(file, startLine, endLine,
tokenBag)`. Line numbers come from the parser/extractor phase.

**False Positive Mitigation:**
- The 70% threshold balances precision (86-91%) and recall (86-100%)
- Sub-block filtering inherently reduces spurious matches
- Token position filtering provides early termination

**Performance:** 250 MLOC in ~1.5 days on 12GB RAM workstation. 3x faster than
CCFinderX.

**Source:** [GitHub](https://github.com/Mondego/SourcererCC) |
[ICSE 2016 Paper](https://ieeexplore.ieee.org/document/7886988/)

---

### 5. NiCad

**Algorithm:** Hybrid AST-extraction + text-line comparison (Longest Common Subsequence).
**Languages:** C, Java, Python, C#, Ruby, Rust, Swift, PHP, others (via TXL grammars).
**Clone Types:** Type-1, Type-2, Type-3 (near-miss).

**Fragment Extraction:** Uses TXL source transformation system with "island grammars"
to extract fragments at a chosen granularity (functions, blocks, statements). Extraction
is AST-aware -- it uses a real parser for the target language. Fragment boundaries
align with syntactic constructs (function definitions, method declarations, etc.).

Granularity is configured by plugin naming convention: e.g., `java-functions-extract.txl`
extracts Java functions.

**Matching Algorithm:** After extraction and normalization, NiCad uses **line-by-line
text comparison** with **Longest Common Subsequence (LCS)** to compute similarity.
This is NOT token-based -- it compares pretty-printed, normalized lines of code.

Near-miss threshold: configurable. `0.00` = exact clones, `0.30` = up to 30% different
lines (default).

**Normalization (the key innovation):**
Three-step pipeline:
1. **Flexible Pretty-Printing**: Standardize formatting. Break statements into parts
   so each semantic unit is on its own line. This enables line-based diff to detect
   statement-level changes.
2. **Blind Renaming**: Replace ALL identifiers with a uniform placeholder (e.g., `$ID`).
   Exposes Type-2 similarity. Example: `nicad functions java MyProject blindrename`
3. **Consistent Renaming**: Replace identifiers systematically so that consistently
   mapped variable names in two fragments are recognized as equivalent. More precise
   than blind renaming -- catches true parameterized clones without over-generalizing.
4. **Filtering/Abstraction**: Remove declarations, annotations, or irrelevant features
   before comparison.

**Line Number Tracking:** Extracted fragments retain their original file path and line
range. Pretty-printing and normalization are applied to copies -- original positions
are preserved.

**False Positive Mitigation:**
- Near-miss threshold (default 0.30) controls sensitivity
- Pretty-printing breaks code into comparable lines (reduces formatting noise)
- Plugin architecture allows language-specific noise removal
- Blind vs. consistent renaming lets user choose precision level

**Performance:** Handles 60+ MLOC in 2GB RAM. Parsing/extraction is the bottleneck
but needs to be done only once per system.

**Source:** [ICPC 2008 Paper](https://ieeexplore.ieee.org/document/4556129/) |
[GitHub](https://github.com/CordyJ/Open-NiCad)

---

### 6. Duplo

**Algorithm:** Line-based string matching (Ducasse/Rieger/Demeyer approach from Duploc).
**Languages:** C, C++, Java, C#, VB.NET + any text format.
**Clone Types:** Type-1 (primarily).

**Fragment Extraction:** No AST. Entire files are processed as sequences of lines.
Language-specific preprocessing strips comments, preprocessor directives, and using
statements. Fragments are contiguous runs of matching lines.

**Matching Algorithm:** Dynamic Pattern Matching (DPM) on line sequences. Essentially
builds a match matrix comparing every pair of lines between/within files and finds
diagonal runs (contiguous matching sequences). This is the scatter-plot matrix approach
from Ducasse et al. (1999).

**Normalization:** Minimal. Whitespace normalization within lines. Comment and
preprocessor directive removal per language. No identifier or literal normalization.

**Defaults:** Minimum 6 matching lines (configurable).

**Line Number Tracking:** Direct -- each line in the file has its line number, and
matching runs produce start/end line ranges.

**False Positive Mitigation:**
- Minimum line threshold
- Comment/preprocessor stripping reduces noise
- Language-specific filters (e.g., `CstyleCommentsFilter`)

**Performance:** 10K+ lines/second single-threaded. Multithreading support (`-j` flag)
with near-linear speedup. Very fast for small-medium codebases.

**Source:** [GitHub](https://github.com/dlidstrom/Duplo) |
[SourceForge](https://duplo.sourceforge.net/)

---

### 7. Rust Ecosystem Options

#### duplicate_code (crate)
- Scans directories for duplicate code segments
- Hosted on GitLab (`DeveloperC/duplicate_code`)
- Minimal documentation, unclear algorithm
- Not widely adopted

#### CCDetect-lsp (Java, not Rust, but highly relevant)
- **Algorithm:** Suffix array + LCP array over token sequences
- **Tree-sitter integration** for incremental parsing (any language)
- **Fragment extraction via tree-sitter queries**: e.g., `(function_item) @function`
  for Rust, `(method_declaration) @method` for Java
- **Incremental detection**: Dynamic suffix array updates on edit (O(edit size) not
  O(codebase size))
- **Blind node support**: Configure AST node types (identifiers, literals) to be
  "blinded" (replaced with uniform placeholder) for Type-2 detection
- **Token threshold**: Configurable (e.g., 100 tokens)
- Based on master thesis: "Incremental clone detection for IDEs using dynamic suffix
  arrays" (University of Oslo, 2023)

**Source:** [GitHub](https://github.com/jakobkhansen/CCDetect-lsp) |
[Thesis](https://www.mn.uio.no/ifi/english/research/groups/psy/completedmasters/2023/Hansen/)

#### superdiff (crate)
- Finds similar blocks in codebases
- Designed for "slightly different" copy-pasted code
- Simpler approach than full clone detection

---

## Decision Matrix

### Decision 1: Fragment Extraction Strategy

| Decision | jscpd | PMD CPD | SourcererCC | NiCad | Duplo | CCDetect-lsp | **Our Choice** | **Rationale** |
|---|---|---|---|---|---|---|---|---|
| **Fragment unit** | Sliding window over tokens | Flat token stream | Function/block/file (configurable) | Function/block (AST-extracted) | Lines (no structure) | Function/block (tree-sitter query) | **Function/block via tree-sitter query** | We already have tree-sitter. Function-level gives semantic boundaries. Block-level is fallback for languages without clear functions. |
| **How boundaries determined** | Token count window | No boundaries | Language-specific parser | TXL grammar extraction | Line-by-line | Tree-sitter query capture | **Tree-sitter queries per language** | CCDetect-lsp proves this works. Queries like `(function_item) @fn` for Rust, `(function_declaration) @fn` for JS/TS. |
| **Multi-granularity** | No | No | Yes (file/method/block) | Yes (function/block) | No | Yes (via query) | **Yes -- function primary, block fallback** | SourcererCC and NiCad both show multi-granularity improves recall. |

### Decision 2: Matching Algorithm

| Decision | jscpd | PMD CPD | SourcererCC | NiCad | Duplo | CCDetect-lsp | **Our Choice** | **Rationale** |
|---|---|---|---|---|---|---|---|---|
| **Core algorithm** | Rabin-Karp rolling hash | Karp-Rabin rolling hash | Bag-of-tokens overlap | LCS on pretty-printed lines | Line match matrix (DPM) | Suffix array + LCP | **Two-phase: hash buckets + token overlap** | Phase 1: Rabin-Karp hash for O(1) Type-1/2 detection (exact match after normalization). Phase 2: Bag-of-tokens overlap for Type-3 (SourcererCC approach). Suffix array is optimal but complex; save for v3. |
| **Type-1 detection** | Hash match | Hash match | 100% overlap | LCS = 1.0 | All lines match | SA match ≥ threshold | **Hash equality on normalized token sequence** | Fastest possible. FNV/xxHash on normalized tokens. |
| **Type-2 detection** | N/A (same hash after normalization) | Hash match after identifier replacement | High overlap with different token values | LCS after blind renaming | N/A | SA match after blinding | **Hash equality after blind normalization** | Apply blind renaming to tokens, then hash. Same as PMD CPD approach. |
| **Type-3 detection** | Not supported | Not supported | Overlap similarity ≥ threshold | LCS ≥ (1 - near_miss_threshold) | Not supported | SA-based (limited) | **Token bag overlap similarity ≥ 0.70** | SourcererCC's approach is proven at scale (250 MLOC). Token bags are order-independent, which naturally handles statement reordering. |

### Decision 3: Normalization Approach

| Decision | jscpd | PMD CPD | SourcererCC | NiCad | Duplo | CCDetect-lsp | **Our Choice** | **Rationale** |
|---|---|---|---|---|---|---|---|---|
| **Identifier handling** | None | Blind replacement (optional) | Bag inherently handles | Blind OR consistent renaming | None | Blind (configurable node types) | **Blind replacement by default, consistent renaming optional** | Blind is simpler and sufficient for most cases. PMD CPD and NiCad both use it. Consistent renaming is more precise but adds complexity -- offer as option. |
| **Literal handling** | None | Blind replacement (optional) | Bag inherently handles | Blind replacement | None | Blind (configurable) | **Blind replacement by default** | Same rationale as identifiers. |
| **How normalization works** | Token mode (strict/mild/weak) | Pre-processing before hash | N/A (bag approach) | TXL transformation rules | Whitespace only | AST node type blinding | **Tree-sitter node kind check during tokenization** | During AST walk, check node kind. If `identifier` → emit `$ID`. If `string_literal`/`number_literal` → emit `$LIT`. This is what CCDetect-lsp does and it's clean. |
| **Comment handling** | Mode-dependent | Lexer strips comments | Parser strips comments | Pretty-printing removes | Language-specific filter | Tree-sitter (comments are nodes) | **Skip comment nodes during AST walk** | Tree-sitter exposes comments as named nodes. Simply skip them during token extraction. Zero-cost. |

### Decision 4: Line Number Tracking

| Decision | jscpd | PMD CPD | SourcererCC | NiCad | Duplo | CCDetect-lsp | **Our Choice** | **Rationale** |
|---|---|---|---|---|---|---|---|---|
| **Tracking method** | Prism.js token positions | Token carries file+line | Parser stores (file, start, end) | Fragment retains original position | Direct line numbers | Tree-sitter node positions | **Tree-sitter `Node::start_position()` / `Node::end_position()`** | Tree-sitter provides byte offset, row, and column for every node. Zero extra work needed. |
| **Precision** | Token-level | Token-level | Block-level | Line-level | Line-level | Token-level | **Token-level (row:col from tree-sitter)** | We get this for free. More precise than line-level. |
| **Remapping needed?** | Yes (token → source) | Yes (token index → source) | No (stored at extraction) | No (preserved through normalization) | No (direct) | No (tree-sitter provides) | **No remapping needed** | Tree-sitter nodes carry positions. We store them during extraction. |

### Decision 5: Minimum Thresholds

| Decision | jscpd | PMD CPD | SourcererCC | NiCad | Duplo | CCDetect-lsp | **Our Choice** | **Rationale** |
|---|---|---|---|---|---|---|---|---|
| **min_tokens default** | 50 | 100 (recommended) | N/A (bag-based) | N/A (line-based) | N/A | 100 | **50 tokens** | jscpd's 50 is a good balance. PMD's 100 is conservative (designed for Java with verbose syntax). For multi-language support, 50 catches more clones without excessive noise. Existing clones.rs uses 25 which is too low. |
| **min_lines default** | 5 | N/A | N/A | N/A | 6 | N/A | **5 lines** | Industry consensus. jscpd uses 5, Duplo uses 6. Our existing clones.rs uses 5, which is correct. |
| **similarity threshold** | N/A (exact only) | N/A (exact only) | 0.70 (70%) | 0.70 (30% diff) | N/A | N/A | **0.70 (70% overlap)** | Both SourcererCC and NiCad converge on 0.70 as optimal. Our existing clones.rs already uses 0.70. |

### Decision 6: False Positive Mitigation

| Decision | jscpd | PMD CPD | SourcererCC | NiCad | Duplo | CCDetect-lsp | **Our Choice** | **Rationale** |
|---|---|---|---|---|---|---|---|---|
| **Annotation filtering** | No | Yes (Java) | No | Yes (via normalization) | No | No | **Yes -- skip annotation/attribute nodes** | Tree-sitter exposes annotations as nodes. Skip `attribute_item` (Rust), `decorator` (Python), `annotation` (Java). Reduces false positives from framework boilerplate. |
| **Import/using filtering** | No | Yes (C#) | No | Yes (via filtering) | Yes | No | **Yes -- skip import/use statements** | Import blocks are always similar. Filter during extraction. |
| **Generated file exclusion** | Via ignore patterns | Via exclusion files | No | No | No | No | **Yes -- exclude `*.pb.go`, `*_generated.*`, etc.** | Already in existing ClonesOptions. Keep it. |
| **Suppression comments** | No | Yes (`CPD-OFF`/`CPD-ON`) | No | No | No | No | **Future -- not for v2 MVP** | Nice to have but not essential. Can add `// clone-ignore` support later. |
| **Subsumption filtering** | No | No | Sub-block overlap | No | No | No | **Yes -- remove subsumed pairs** | If fragment A contains fragment B, and both match fragment C, only report the larger match. Reduces noise significantly. |

---

## Detailed Implementation Recommendations

### Token Extraction Pipeline

Based on the prior art, the recommended pipeline is:

```
Source File
    → tree-sitter parse → AST
    → tree-sitter query → fragment nodes (functions, blocks)
    → AST walk per fragment → token sequence
        - Skip comment nodes
        - Skip import/use nodes
        - Skip annotation/decorator nodes
        - Normalize identifiers → "$ID"
        - Normalize literals → "$LIT"
    → (file, start_line, end_line, normalized_tokens, raw_tokens)
```

This combines:
- **NiCad's** AST-based extraction with language-specific granularity
- **CCDetect-lsp's** tree-sitter query approach for fragment boundaries
- **PMD CPD's** blind normalization for Type-2 detection
- **SourcererCC's** bag-of-tokens for Type-3 comparison

### Two-Phase Detection

**Phase 1: Exact/Near-Exact (Type-1 and Type-2)**
```
For each fragment:
    hash = xxhash(normalized_token_sequence)
    bucket[hash].push(fragment)
For each bucket with 2+ fragments:
    Emit Type-1 or Type-2 clone pair (verify with direct comparison)
```

This is essentially what jscpd and PMD CPD do. O(n) where n = number of fragments.

**Phase 2: Near-Miss (Type-3)**
```
For each pair of fragments in same hash neighborhood:
    bag_a = multiset(normalized_tokens of fragment_a)
    bag_b = multiset(normalized_tokens of fragment_b)
    overlap = |bag_a ∩ bag_b| / max(|bag_a|, |bag_b|)
    if overlap >= threshold:
        Emit Type-3 clone pair with similarity = overlap
```

Optimization (from SourcererCC):
- Build partial inverted index (index only 30% of tokens per fragment)
- Use token position filtering for early termination

### Fragment Boundary Queries

Per-language tree-sitter queries for fragment extraction:

```
Rust:       (function_item) @fn
            (impl_item (function_item) @method)
Python:     (function_definition) @fn
            (class_definition (function_definition) @method)
JavaScript: (function_declaration) @fn
            (method_definition) @method
            (arrow_function) @fn
TypeScript: (function_declaration) @fn
            (method_definition) @method
            (method_signature) @method
            (abstract_method_signature) @method
            (arrow_function) @fn
Go:         (function_declaration) @fn
            (method_declaration) @method
Java:       (method_declaration) @method
            (constructor_declaration) @constructor
C/C++:      (function_definition) @fn
```

### Token Categories for Normalization

Based on tree-sitter node kinds, classify tokens:

| Category | Example Node Kinds | Normalization |
|---|---|---|
| Identifier | `identifier`, `field_identifier`, `type_identifier` | → `$ID` (blind) |
| String Literal | `string_literal`, `raw_string_literal`, `template_string` | → `$STR` |
| Number Literal | `integer_literal`, `float_literal` | → `$NUM` |
| Boolean Literal | `true`, `false` | → `$BOOL` |
| Keyword | `fn`, `let`, `if`, `return`, `for`, `while` | Keep as-is |
| Operator | `+`, `-`, `*`, `==`, `!=`, `&&` | Keep as-is |
| Punctuation | `{`, `}`, `(`, `)`, `;`, `,` | Keep as-is |
| Comment | `line_comment`, `block_comment` | Skip entirely |
| Annotation | `attribute_item`, `decorator` | Skip entirely |
| Import | `use_declaration`, `import_statement` | Skip entirely |

---

## Performance Estimates

Based on prior art benchmarks:

| Operation | Expected Performance | Basis |
|---|---|---|
| Tree-sitter parse | ~100K lines/sec | tree-sitter benchmarks |
| Fragment extraction | ~50K functions/sec | Linear AST walk |
| Token normalization | ~500K tokens/sec | Simple node-kind switch |
| Phase 1 (hash bucketing) | ~100K fragments/sec | HashMap insert + lookup |
| Phase 2 (bag overlap) | ~1K-10K pairs/sec | Depends on fragment size |
| Total for 100K LOC | < 5 seconds | Conservative estimate |

---

## Open Questions

1. **Consistent renaming**: Should we implement alpha-renaming (parameterized matching)
   in v2, or defer to v3? NiCad shows it improves Type-2 precision, but blind renaming
   is sufficient for most use cases.

2. **Cross-file vs. within-file**: SourcererCC handles both. Our existing code has
   `include_within_file` option. Should within-file be default-on or default-off?
   PMD CPD and jscpd both default to including within-file clones.

3. **Suffix array for v3**: CCDetect-lsp's suffix array approach is theoretically
   superior (especially for incremental updates). Worth investigating for a future
   version where we want LSP-integrated real-time clone detection.

4. **Block-level granularity**: For languages with many small functions (Go, Rust),
   function-level might miss large block clones inside different functions. Should
   we extract both function-level and significant-block-level fragments?

---

## Sources

1. [rust-code-analysis - GitHub](https://github.com/mozilla/rust-code-analysis)
2. [jscpd - GitHub](https://github.com/kucherenko/jscpd) | [npm](https://www.npmjs.com/package/jscpd)
3. [PMD CPD Documentation](https://docs.pmd-code.org/pmd-doc-6.55.0/pmd_userdocs_cpd.html) | [GitHub](https://github.com/pmd/pmd)
4. [SourcererCC - ICSE 2016](https://ieeexplore.ieee.org/document/7886988/) | [GitHub](https://github.com/Mondego/SourcererCC)
5. [NiCad - ICPC 2008](https://ieeexplore.ieee.org/document/4556129/) | [Open-NiCad GitHub](https://github.com/CordyJ/Open-NiCad)
6. [Duplo - GitHub](https://github.com/dlidstrom/Duplo) | [SourceForge](https://duplo.sourceforge.net/)
7. [CCDetect-lsp - GitHub](https://github.com/jakobkhansen/CCDetect-lsp) | [Thesis](https://www.mn.uio.no/ifi/english/research/groups/psy/completedmasters/2023/Hansen/)
8. [duplicate_code crate](https://lib.rs/crates/duplicate_code)
9. [Ducasse, Rieger, Demeyer - "A language independent approach for detecting duplicated code" (ICSM 1999)](https://onlinelibrary.wiley.com/doi/10.1002/smr.317)
10. [Kovalenko et al. - "Multi-threshold token-based code clone detection"](https://arxiv.org/abs/2002.05204)
11. [SourcererCC Paper (full)](https://clones.usask.ca/pubfiles/articles/SajnaniSourcererCCICSE2016.pdf)
12. [NiCad Tool Paper (ICPC 2011)](https://www.cs.usask.ca/~croy/papers/2011/CR-NiCad-Tool-ICPC11.pdf)
13. [String Similarity via Greedy String Tiling and Running Karp-Rabin Matching (Wise)](https://www.researchgate.net/profile/Michael_Wise/publication/262763983_String_Similarity_via_Greedy_String_Tiling_and_Running_Karp-Rabin_Matching)
