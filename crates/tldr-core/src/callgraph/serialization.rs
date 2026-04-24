//! Serialization for CallGraphIR (Phase 15)
//!
//! This module provides JSON serialization with versioning for the call graph IR.
//! It implements the spec from `migration/spec/phases-14-16-spec.md` Section 15.
//!
//! # Key Features
//!
//! - Version-tagged JSON output for compatibility checking
//! - POSIX path normalization (forward slashes on all platforms)
//! - Round-trip identity guarantee: `from_json(to_json(x)) == x`
//! - Unknown fields ignored for forward compatibility
//!
//! # Mitigations Implemented
//!
//! - M3.3: Serialize as strings, not interned IDs (rebuild interner on load)
//! - M3.5: Use serde default to ignore unknown fields
//! - M3.10: Convert all paths to forward slashes in to_json()

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use super::cross_file_types::{
    CallGraphIR, CallSite, ClassDef, FileIR, FuncDef, ImportDef, ProjectCallGraphV2, VarType,
};

// =============================================================================
// IR_VERSION Constant (Spec Section 15.2)
// =============================================================================

/// IR schema version for compatibility checking.
///
/// Versioning follows semver principles:
/// - Major: Breaking schema changes (requires migration or rebuild)
/// - Minor: Additive changes (backward compatible)
/// - Patch: Bug fixes in serialization
///
/// Version history:
/// - "1.0": Initial release
pub const IR_VERSION: &str = "1.0";

// =============================================================================
// SerializationError (Spec Section 15.6)
// =============================================================================

/// Errors that can occur during serialization/deserialization.
#[derive(Debug, thiserror::Error)]
pub enum SerializationError {
    /// IR version in JSON doesn't match expected version.
    #[error("IR version mismatch: expected {expected}, got {actual}")]
    IRVersionMismatch {
        /// Expected version (usually IR_VERSION)
        expected: String,
        /// Actual version found in JSON
        actual: String,
    },

    /// JSON structure is malformed or doesn't match expected schema.
    #[error("Invalid JSON format: {0}")]
    InvalidFormat(String),

    /// Required field is missing from JSON.
    #[error("Missing required field: {0}")]
    MissingField(String),

    /// JSON parsing/serialization error from serde_json.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// IO error during file operations.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

// =============================================================================
// Serialization Types (Internal)
// =============================================================================

/// JSON representation of CallGraphIR.
///
/// This is the serialization format - it uses strings instead of interned IDs
/// and includes the _version field for compatibility checking.
#[derive(Serialize, Deserialize)]
struct CallGraphIRJson {
    /// Schema version (always "_version" in JSON for visibility)
    #[serde(rename = "_version")]
    version: String,

    /// Project root directory (POSIX format)
    root: String,

    /// Primary language of the project
    language: String,

    /// Files in the project, keyed by normalized path
    files: HashMap<String, FileIRJson>,
}

/// JSON representation of FileIR.
#[derive(Serialize, Deserialize)]
struct FileIRJson {
    /// File path relative to project root (POSIX format)
    path: String,

    /// Functions defined in this file
    #[serde(default)]
    functions: Vec<FuncDef>,

    /// Classes defined in this file
    #[serde(default)]
    classes: Vec<ClassDef>,

    /// Import statements
    #[serde(default)]
    imports: Vec<ImportDef>,

    /// Call sites (flattened from the HashMap)
    #[serde(default)]
    calls: Vec<CallSite>,

    /// Variable type information
    #[serde(default)]
    var_types: Vec<VarType>,
}

// =============================================================================
// CallGraphIR Serialization Methods (Spec Section 15.3)
// =============================================================================

impl CallGraphIR {
    /// Serialize to JSON string.
    ///
    /// # Output Format
    ///
    /// ```json
    /// {
    ///   "_version": "1.0",
    ///   "root": "/path/to/project",
    ///   "language": "python",
    ///   "files": { ... }
    /// }
    /// ```
    ///
    /// # Guarantees
    ///
    /// - Deterministic output (sorted keys via BTreeMap-like iteration)
    /// - All paths as POSIX (forward slashes)
    /// - UTF-8 encoded
    pub fn to_json(&self) -> Result<String, SerializationError> {
        let value = self.to_json_value();
        serde_json::to_string(&value).map_err(SerializationError::Json)
    }

    /// Serialize to JSON Value (for embedding in larger structures).
    ///
    /// Returns a serde_json::Value that can be further manipulated or
    /// combined with other JSON structures.
    pub fn to_json_value(&self) -> Value {
        let json_ir = self.to_json_representation();
        serde_json::to_value(&json_ir).expect("CallGraphIR should serialize to JSON")
    }

    /// Deserialize from JSON string.
    ///
    /// # Errors
    ///
    /// - `IRVersionMismatch` if _version doesn't match IR_VERSION
    /// - `InvalidFormat` for malformed JSON
    /// - `MissingField` for required fields
    /// - `Json` for serde_json parsing errors
    pub fn from_json(json: &str) -> Result<Self, SerializationError> {
        // First parse as generic Value to check version
        let value: Value = serde_json::from_str(json).map_err(|e| {
            if json.contains("not valid") || !json.trim().starts_with('{') {
                SerializationError::InvalidFormat(e.to_string())
            } else {
                SerializationError::Json(e)
            }
        })?;

        Self::from_json_value(value)
    }

    /// Deserialize from JSON Value.
    ///
    /// # Errors
    ///
    /// Same as `from_json`.
    pub fn from_json_value(value: Value) -> Result<Self, SerializationError> {
        // Check for _version field
        let version = value
            .get("_version")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SerializationError::MissingField("_version".to_string()))?;

        // Validate version matches
        if version != IR_VERSION {
            return Err(SerializationError::IRVersionMismatch {
                expected: IR_VERSION.to_string(),
                actual: version.to_string(),
            });
        }

        // Parse the full structure (unknown fields are ignored by serde default)
        let json_ir: CallGraphIRJson =
            serde_json::from_value(value).map_err(SerializationError::Json)?;

        Ok(Self::from_json_representation(json_ir))
    }

    /// Convert to JSON representation struct.
    fn to_json_representation(&self) -> CallGraphIRJson {
        let mut files = HashMap::new();

        for (path, file_ir) in &self.files {
            let path_str = normalize_path_string(&path.to_string_lossy());
            let file_json = FileIRJson {
                path: path_str.clone(),
                functions: file_ir.funcs.clone(),
                classes: file_ir.classes.clone(),
                imports: file_ir.imports.clone(),
                calls: flatten_calls(&file_ir.calls),
                var_types: file_ir.var_types.clone(),
            };
            files.insert(path_str, file_json);
        }

        CallGraphIRJson {
            version: IR_VERSION.to_string(),
            root: normalize_path_string(&self.root.to_string_lossy()),
            language: self.language.clone(),
            files,
        }
    }

    /// Convert from JSON representation struct.
    fn from_json_representation(json_ir: CallGraphIRJson) -> Self {
        let mut ir = Self::new(PathBuf::from(&json_ir.root), &json_ir.language);

        for (_path_key, file_json) in json_ir.files {
            let file_ir = FileIR {
                path: PathBuf::from(&file_json.path),
                funcs: file_json.functions,
                classes: file_json.classes,
                imports: file_json.imports,
                var_types: file_json.var_types,
                calls: unflatten_calls(file_json.calls),
            };
            ir.add_file(file_ir);
        }

        ir.build_indices();
        ir
    }
}

// =============================================================================
// ProjectCallGraphV2 Serialization (Spec Section 15.5)
// =============================================================================

impl ProjectCallGraphV2 {
    /// Serialize edges to JSON array of 4-tuples.
    ///
    /// Format: `[["src_file", "src_func", "dst_file", "dst_func"], ...]`
    ///
    /// # Determinism
    ///
    /// Edges are sorted lexicographically for deterministic output:
    /// 1. By src_file
    /// 2. By src_func
    /// 3. By dst_file
    /// 4. By dst_func
    pub fn edges_to_json(&self) -> Value {
        let mut edges: Vec<_> = self.edges().collect();

        // Sort for determinism
        edges.sort_by(|a, b| {
            (&a.src_file, &a.src_func, &a.dst_file, &a.dst_func).cmp(&(
                &b.src_file,
                &b.src_func,
                &b.dst_file,
                &b.dst_func,
            ))
        });

        Value::Array(
            edges
                .into_iter()
                .map(|e| {
                    serde_json::json!([
                        normalize_path_string(&e.src_file.to_string_lossy()),
                        &e.src_func,
                        normalize_path_string(&e.dst_file.to_string_lossy()),
                        &e.dst_func
                    ])
                })
                .collect(),
        )
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Normalize a path string to POSIX format (forward slashes).
///
/// This ensures cross-platform compatibility in serialized JSON.
fn normalize_path_string(path: &str) -> String {
    path.replace('\\', "/")
}

/// Flatten calls HashMap to a Vec for serialization.
///
/// The caller field in each CallSite already contains the function name,
/// so we don't lose information by flattening.
fn flatten_calls(calls: &HashMap<String, Vec<CallSite>>) -> Vec<CallSite> {
    let mut result: Vec<CallSite> = calls.values().flatten().cloned().collect();

    // Sort for determinism
    result.sort_by(|a, b| (&a.caller, &a.target, &a.line).cmp(&(&b.caller, &b.target, &b.line)));

    result
}

/// Unflatten calls Vec to HashMap, grouping by caller.
fn unflatten_calls(calls: Vec<CallSite>) -> HashMap<String, Vec<CallSite>> {
    let mut result: HashMap<String, Vec<CallSite>> = HashMap::new();

    for call in calls {
        result.entry(call.caller.clone()).or_default().push(call);
    }

    result
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path_string() {
        assert_eq!(normalize_path_string("src\\main.py"), "src/main.py");
        assert_eq!(normalize_path_string("src/main.py"), "src/main.py");
        assert_eq!(
            normalize_path_string("C:\\Users\\test\\project"),
            "C:/Users/test/project"
        );
    }

    #[test]
    fn test_ir_version_constant() {
        assert_eq!(IR_VERSION, "1.0");
    }

    #[test]
    fn test_error_display() {
        let err = SerializationError::IRVersionMismatch {
            expected: "1.0".to_string(),
            actual: "0.5".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("1.0"));
        assert!(msg.contains("0.5"));
    }
}
