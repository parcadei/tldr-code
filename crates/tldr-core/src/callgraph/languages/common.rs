//! Shared helpers for language call extraction walkers.
//!
//! These utilities intentionally stay small and generic so language handlers can
//! compose them without coupling to each other's AST node kinds.

use std::collections::HashMap;

use crate::callgraph::cross_file_types::CallSite;

/// Build a qualified name from an optional prefix.
pub(crate) fn qualify_name(prefix: Option<&str>, name: &str, separator: &str) -> String {
    match prefix {
        Some(value) if !value.is_empty() => format!("{value}{separator}{name}"),
        _ => name.to_string(),
    }
}

/// Return the receiver portion of a qualified target.
///
/// For `Foo.bar` this returns `Some("Foo")`.
pub(crate) fn receiver_from_target(target: &str, separator: char) -> Option<String> {
    target
        .rfind(separator)
        .map(|idx| target[..idx].to_string())
        .filter(|receiver| !receiver.is_empty())
}

/// Extend an entry with calls when calls are present.
pub(crate) fn extend_calls_if_any(
    calls_by_func: &mut HashMap<String, Vec<CallSite>>,
    caller: impl Into<String>,
    calls: Vec<CallSite>,
) {
    if calls.is_empty() {
        return;
    }
    calls_by_func
        .entry(caller.into())
        .or_default()
        .extend(calls);
}

/// Insert or replace calls for a caller when calls are present.
pub(crate) fn insert_calls_if_any(
    calls_by_func: &mut HashMap<String, Vec<CallSite>>,
    caller: impl Into<String>,
    calls: Vec<CallSite>,
) {
    if calls.is_empty() {
        return;
    }
    calls_by_func.insert(caller.into(), calls);
}
