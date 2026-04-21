//! Security analysis module - Phase 8
//!
//! This module provides security vulnerability detection:
//! - Secret scanning (API keys, passwords, private keys)
//! - Vulnerability detection via taint analysis (SQL injection, XSS, command injection)
//!
//! # References
//! - OWASP Top 10
//! - CWE/SANS Top 25

pub mod ast_utils;
pub mod secrets;
pub mod taint;
pub mod vuln;

// Taint analysis tests (CFG-based taint tracking)
#[cfg(test)]
mod taint_tests;

pub use secrets::{scan_secrets, SecretFinding, SecretsReport, Severity};
pub use taint::{
    compute_taint, compute_taint_with_tree, detect_sanitizer_ast, detect_sinks_ast,
    detect_sources_ast, SanitizerType, TaintFlow, TaintInfo, TaintSink, TaintSinkType, TaintSource,
    TaintSourceType,
};
pub use vuln::{scan_vulnerabilities, VulnFinding, VulnReport, VulnType};
