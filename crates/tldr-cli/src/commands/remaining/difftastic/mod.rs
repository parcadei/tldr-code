//! Vendored difftastic core + adapters for L1/L2 diff.
//!
//! Vendored from difftastic with modifications:
//! - Import paths rewritten from crate:: to super::
//! - Logging removed (no log/humansize dependency)
//! - Display-layer code stripped (MatchedPos, split_atom_words, etc.)
//! - sliders.rs simplified (prefer_outer always false for our 18 languages)

pub mod changes;
pub mod hash;
pub mod lang_config;
pub mod lcs_diff;
pub mod lifetime_proto;
pub mod stack;
pub mod syntax;

// Phase 3: Core algorithm
pub mod dijkstra;
pub mod graph;
pub mod sliders;
pub mod unchanged;

// Phase 4: Tree-sitter to Syntax adapter
pub mod ts_to_syntax;

// Phase 5: ChangeMap to DiffReport adapter
pub mod changemap_to_report;
