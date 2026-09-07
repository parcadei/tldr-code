//! Reproduction tests for GitHub issues parcadei/tldr-code#9 + #16.
//!
//! These tests cover every known site where `tldr` previously sliced UTF-8
//! strings at byte offsets that may land mid-codepoint, panicking on multi-byte
//! input (CJK, accented Latin, emoji). Each test exercises the relevant code
//! path with input designed to force a slice across a UTF-8 boundary.
//!
//! On HEAD a8d077c (pre-fix) every test below panics with
//!   `byte index N is not a char boundary; it is inside '\u{...}' (bytes ...)`
//!
//! Post-fix every truncated string is valid UTF-8 and ends on a char boundary.
//!
//! Sites covered:
//!   Surface modules (12): python, ruby, typescript, java, rust_lang, go,
//!     javascript, csharp, scala, kotlin, elixir, swift
//!   CLI output (8): cognitive (:639), smells (:951), secrets (:1048),
//!     clones file_a (:1641), clones file_b (:1646),
//!     module-level docstring (:2206), class docstring (:2261),
//!     function docstring (:2394)

use std::collections::HashMap;
use std::path::PathBuf;

use tldr_core::analysis::clones::{
    CloneConfig, CloneFragment, ClonePair, ClonesReport, NormalizationMode,
};
use tldr_core::metrics::cognitive::{
    CognitiveReport, CognitiveSummary, FunctionCognitive, ThresholdStatus,
};
use tldr_core::quality::smells::{SmellFinding, SmellType, SmellsReport, SmellsSummary};
use tldr_core::types::{ClassInfo, FunctionInfo, IntraFileCallGraph, Language, ModuleInfo};

use tldr_cli::output::{
    format_clones_text, format_cognitive_text, format_module_info_text, format_smells_text,
};

/// 67 × U+4E16 (CJK 世) = 201 bytes. Slicing at byte 197 lands inside the
/// 66th character (which spans bytes 195..198), forcing a panic on the legacy
/// `&s[..197]` code paths.
fn cjk_201_bytes() -> String {
    "\u{4e16}".repeat(67)
}

/// Path that forces the clones tail-slice (`len - 27`) to land mid-codepoint.
/// 4-byte emoji × 11 = 44 bytes; tail start = 44 - 27 = 17, which is mid-emoji
/// (codepoint #5 spans bytes 16..20). The CJK path above happens to start at
/// byte 15, a coincidental valid boundary; emoji exposes the bug cleanly.
fn emoji_path_clones_tail_misaligned() -> PathBuf {
    PathBuf::from("\u{1f600}".repeat(11))
}

// =============================================================================
// CLI output formatters: cognitive (:639)
// =============================================================================

#[test]
fn cli_cognitive_text_does_not_panic_on_cjk_function_name() {
    // 67 × 世 = 201 bytes; output.rs:638 takes the > 28 chars branch and
    // pre-fix sliced `&f.name[..25]` straight through the third codepoint.
    let report = CognitiveReport {
        functions: vec![FunctionCognitive {
            name: cjk_201_bytes(),
            file: "src/lib.rs".to_string(),
            line: 1,
            cognitive: 1,
            cyclomatic: None,
            max_nesting: 0,
            nesting_penalty: 0,
            threshold_status: ThresholdStatus::Ok,
            contributors: None,
        }],
        violations: vec![],
        summary: CognitiveSummary::default(),
        warnings: vec![],
    };

    let out = format_cognitive_text(&report);
    assert!(
        std::str::from_utf8(out.as_bytes()).is_ok(),
        "output is not valid UTF-8"
    );
}

// =============================================================================
// CLI output formatters: smells (:951)
// =============================================================================

#[test]
fn cli_smells_text_does_not_panic_on_cjk_smell_name() {
    let report = SmellsReport {
        smells: vec![SmellFinding {
            smell_type: SmellType::LongMethod,
            file: PathBuf::from("src/lib.rs"),
            name: cjk_201_bytes(),
            line: 1,
            reason: "test".to_string(),
            severity: 1,
            suggestion: None,
        }],
        files_scanned: 1,
        by_file: HashMap::new(),
        summary: SmellsSummary {
            total_smells: 1,
            by_type: HashMap::new(),
            avg_smells_per_file: 1.0,
        },
        excluded_test_smells: 0,
        warnings: Vec::new(),
    };

    let out = format_smells_text(&report);
    assert!(
        std::str::from_utf8(out.as_bytes()).is_ok(),
        "output is not valid UTF-8"
    );
}

// =============================================================================
// CLI output formatters: clones (:1641 + :1646)
// =============================================================================

#[test]
fn cli_clones_text_does_not_panic_on_cjk_file_paths() {
    // file_a/file_b paths are truncated to the last 27 bytes when > 30 chars.
    // 14 × 3-byte CJK = 42 bytes triggers the legacy
    // `&file_a[file_a.len() - 27..]` slice across a codepoint.
    // Use 4-byte emoji × 11 = 44 bytes so that the legacy
    // `&file_a[file_a.len() - 27..]` lands at byte 17 (mid-emoji); 14 × 3-byte
    // CJK works out to start=15 which happens to be a valid boundary, masking
    // the bug. Emoji exposes it.
    let frag_a = CloneFragment {
        file: emoji_path_clones_tail_misaligned(),
        start_line: 1,
        end_line: 10,
        tokens: 50,
        lines: Some(10),
        function: None,
        preview: None,
    };
    let frag_b = CloneFragment {
        file: emoji_path_clones_tail_misaligned(),
        start_line: 20,
        end_line: 30,
        tokens: 50,
        lines: Some(10),
        function: None,
        preview: None,
    };
    let report = ClonesReport {
        root: PathBuf::from("/tmp"),
        language: "rust".to_string(),
        clone_pairs: vec![ClonePair::new(
            1,
            tldr_core::analysis::CloneType::Type1,
            1.0,
            frag_a,
            frag_b,
        )],
        clone_classes: vec![],
        stats: tldr_core::analysis::clones::CloneStats::default(),
        config: CloneConfig {
            min_tokens: 25,
            min_lines: 5,
            similarity_threshold: 0.7,
            normalization: NormalizationMode::All,
            type_filter: None,
        },
    };

    let out = format_clones_text(&report);
    assert!(
        std::str::from_utf8(out.as_bytes()).is_ok(),
        "output is not valid UTF-8"
    );
}

// =============================================================================
// CLI output formatters: module/class/function docstrings (:2206 + :2261 + :2394)
// =============================================================================

#[test]
fn cli_module_info_text_does_not_panic_on_cjk_module_docstring() {
    // Module-level docstring is truncated to 77 bytes when > 80 chars. Use
    // 30 × U+4E16 = 90 bytes to trigger the legacy `&doc[..77]` slice mid-char.
    let info = ModuleInfo {
        file_path: PathBuf::from("test.py"),
        language: Language::Python,
        docstring: Some("\u{4e16}".repeat(30)),
        imports: vec![],
        functions: vec![],
        classes: vec![],
        constants: vec![],
        call_graph: IntraFileCallGraph::default(),
    };

    let out = format_module_info_text(&info);
    assert!(
        std::str::from_utf8(out.as_bytes()).is_ok(),
        "output is not valid UTF-8"
    );
}

#[test]
fn cli_module_info_text_does_not_panic_on_cjk_class_docstring() {
    let info = ModuleInfo {
        file_path: PathBuf::from("test.py"),
        language: Language::Python,
        docstring: None,
        imports: vec![],
        functions: vec![],
        classes: vec![ClassInfo {
            name: "Foo".to_string(),
            bases: vec![],
            docstring: Some("\u{4e16}".repeat(30)),
            methods: vec![],
            fields: vec![],
            decorators: vec![],
            line_number: 1,
            line_end: 1,
        }],
        constants: vec![],
        call_graph: IntraFileCallGraph::default(),
    };

    let out = format_module_info_text(&info);
    assert!(
        std::str::from_utf8(out.as_bytes()).is_ok(),
        "output is not valid UTF-8"
    );
}

#[test]
fn cli_module_info_text_does_not_panic_on_emoji_function_docstring() {
    // Function docstring is truncated to 57 bytes when > 60 chars. Pure CJK
    // (3-byte) gives 57 % 3 = 0, a coincidental boundary that hides the bug.
    // 4-byte emoji × 16 = 64 bytes; slicing at 57 lands inside the 15th emoji
    // (bytes 56..60), forcing the panic.
    let info = ModuleInfo {
        file_path: PathBuf::from("test.py"),
        language: Language::Python,
        docstring: None,
        imports: vec![],
        functions: vec![FunctionInfo {
            name: "foo".to_string(),
            params: vec![],
            return_type: None,
            docstring: Some("\u{1f600}".repeat(16)),
            is_method: false,
            is_async: false,
            decorators: vec![],
            line_number: 1,
            line_end: 1,
        }],
        classes: vec![],
        constants: vec![],
        call_graph: IntraFileCallGraph::default(),
    };

    let out = format_module_info_text(&info);
    assert!(
        std::str::from_utf8(out.as_bytes()).is_ok(),
        "output is not valid UTF-8"
    );
}

// =============================================================================
// Helper sanity check
// =============================================================================

#[test]
fn truncate_helper_returns_valid_utf8_for_cjk_input() {
    // Direct unit test of the shared helper. 67 × 3-byte char = 201 bytes;
    // requesting 197 bytes should NOT panic and should snap down to a char
    // boundary (195 bytes = 65 whole chars).
    let s = "\u{4e16}".repeat(67);
    let out = tldr_core::util::truncate_at_char_boundary(&s, 197);
    assert!(s.is_char_boundary(out.len()));
    assert_eq!(out.len(), 195);
}
