//! L2 finding extractors for bugbot.
//!
//! Previously contained bespoke extractors (complexity, maintainability).
//! These are now superseded by the TldrDifferentialEngine which invokes
//! `tldr` CLI commands and diffs their JSON outputs.
//!
//! This module is retained as a placeholder for any future finding
//! extractors that cannot be expressed as tldr command diffs.
