//! Core types for API surface extraction (structural contracts).
//!
//! These types represent the machine-readable API surface of a library or package.
//! They are distinct from the behavioral contracts in the CLI `contracts` command
//! (pre/postconditions) -- these are *structural* contracts: function signatures,
//! parameter types, return types, usage examples, and trigger keywords.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Complete API surface for a library or package.
///
/// Contains all public API entries extracted from the package source or type stubs,
/// along with metadata about the package and language.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiSurface {
    /// Package name (e.g., "flask", "json", "serde")
    pub package: String,
    /// Language of the package
    pub language: String,
    /// Total number of API entries
    pub total: usize,
    /// Individual API entries
    pub apis: Vec<ApiEntry>,
}

/// A single public API entry (function, method, class, constant, type alias).
///
/// Each entry represents one callable or referenceable symbol in the package's
/// public API surface, enriched with usage examples and trigger keywords for
/// intent-based retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEntry {
    /// Qualified name (e.g., "json.loads", "flask.Flask.route")
    pub qualified_name: String,
    /// Kind of API
    pub kind: ApiKind,
    /// Module path (e.g., "json", "flask.app")
    pub module: String,
    /// Function/method signature (None for constants, type aliases)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
    /// Docstring (first paragraph only, truncated to ~200 chars)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// Example usage string (e.g., "result = json.loads(s)")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    /// Trigger keywords that map intent to this API
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<String>,
    /// Whether this is a property (vs. method/function)
    #[serde(default)]
    pub is_property: bool,
    /// Return type (if resolvable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    /// Source file and line number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
}

/// Kind of API symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiKind {
    /// Top-level function
    Function,
    /// Instance method
    Method,
    /// Class method (Python `@classmethod`)
    ClassMethod,
    /// Static method (Python `@staticmethod`)
    StaticMethod,
    /// Property accessor (Python `@property`)
    Property,
    /// Class definition
    Class,
    /// Struct definition (Rust, Go)
    Struct,
    /// Trait definition (Rust)
    Trait,
    /// Interface definition (TypeScript, Go)
    Interface,
    /// Enum definition
    Enum,
    /// Module-level constant
    Constant,
    /// Type alias
    TypeAlias,
}

impl std::fmt::Display for ApiKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiKind::Function => write!(f, "function"),
            ApiKind::Method => write!(f, "method"),
            ApiKind::ClassMethod => write!(f, "classmethod"),
            ApiKind::StaticMethod => write!(f, "staticmethod"),
            ApiKind::Property => write!(f, "property"),
            ApiKind::Class => write!(f, "class"),
            ApiKind::Struct => write!(f, "struct"),
            ApiKind::Trait => write!(f, "trait"),
            ApiKind::Interface => write!(f, "interface"),
            ApiKind::Enum => write!(f, "enum"),
            ApiKind::Constant => write!(f, "constant"),
            ApiKind::TypeAlias => write!(f, "type_alias"),
        }
    }
}

/// Function or method signature with typed parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    /// Parameters with full type information
    pub params: Vec<Param>,
    /// Return type annotation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    /// Whether the function is declared async
    #[serde(default)]
    pub is_async: bool,
    /// Whether the function is a generator (yield/yield from)
    #[serde(default)]
    pub is_generator: bool,
}

/// A single parameter with type information, defaults, and variadic markers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Param {
    /// Parameter name
    pub name: String,
    /// Type annotation (e.g., "str", "int", "Optional[str]")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_annotation: Option<String>,
    /// Default value (e.g., "None", "42", "\"hello\"")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Whether this is a variadic parameter (*args / ...rest)
    #[serde(default)]
    pub is_variadic: bool,
    /// Whether this is a keyword parameter (**kwargs)
    #[serde(default)]
    pub is_keyword: bool,
}

/// Source location of an API entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    /// Source file path (relative to package root)
    pub file: PathBuf,
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (0-indexed, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}

/// Result of resolving a package to its source directory.
#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    /// Root directory containing the package source files
    pub root_dir: PathBuf,
    /// Package name
    pub package_name: String,
    /// Whether the source is pure Python (vs. C extension)
    pub is_pure_source: bool,
    /// Names exported via `__all__` (Python), or None if unrestricted
    pub public_names: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_surface_serialize_roundtrip() {
        let surface = ApiSurface {
            package: "json".to_string(),
            language: "python".to_string(),
            total: 2,
            apis: vec![
                ApiEntry {
                    qualified_name: "json.loads".to_string(),
                    kind: ApiKind::Function,
                    module: "json".to_string(),
                    signature: Some(Signature {
                        params: vec![
                            Param {
                                name: "s".to_string(),
                                type_annotation: Some("str".to_string()),
                                default: None,
                                is_variadic: false,
                                is_keyword: false,
                            },
                        ],
                        return_type: Some("Any".to_string()),
                        is_async: false,
                        is_generator: false,
                    }),
                    docstring: Some("Deserialize s to a Python object.".to_string()),
                    example: Some("result = json.loads(\"example\")".to_string()),
                    triggers: vec!["parse".to_string(), "deserialize".to_string(), "load".to_string()],
                    is_property: false,
                    return_type: Some("Any".to_string()),
                    location: Some(Location {
                        file: PathBuf::from("__init__.py"),
                        line: 10,
                        column: None,
                    }),
                },
                ApiEntry {
                    qualified_name: "json.JSONEncoder".to_string(),
                    kind: ApiKind::Class,
                    module: "json".to_string(),
                    signature: None,
                    docstring: Some("Extensible JSON encoder.".to_string()),
                    example: Some("encoder = json.JSONEncoder()".to_string()),
                    triggers: vec!["encode".to_string(), "encoder".to_string()],
                    is_property: false,
                    return_type: None,
                    location: Some(Location {
                        file: PathBuf::from("encoder.py"),
                        line: 1,
                        column: None,
                    }),
                },
            ],
        };

        let json = serde_json::to_string_pretty(&surface).expect("serialize");
        let deser: ApiSurface = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deser.package, "json");
        assert_eq!(deser.language, "python");
        assert_eq!(deser.total, 2);
        assert_eq!(deser.apis.len(), 2);
        assert_eq!(deser.apis[0].qualified_name, "json.loads");
        assert_eq!(deser.apis[0].kind, ApiKind::Function);
        assert_eq!(deser.apis[1].kind, ApiKind::Class);
    }

    #[test]
    fn test_api_kind_display() {
        assert_eq!(ApiKind::Function.to_string(), "function");
        assert_eq!(ApiKind::Method.to_string(), "method");
        assert_eq!(ApiKind::ClassMethod.to_string(), "classmethod");
        assert_eq!(ApiKind::StaticMethod.to_string(), "staticmethod");
        assert_eq!(ApiKind::Property.to_string(), "property");
        assert_eq!(ApiKind::Class.to_string(), "class");
        assert_eq!(ApiKind::Struct.to_string(), "struct");
        assert_eq!(ApiKind::Trait.to_string(), "trait");
        assert_eq!(ApiKind::Interface.to_string(), "interface");
        assert_eq!(ApiKind::Enum.to_string(), "enum");
        assert_eq!(ApiKind::Constant.to_string(), "constant");
        assert_eq!(ApiKind::TypeAlias.to_string(), "type_alias");
    }

    #[test]
    fn test_param_serialize_skip_defaults() {
        let param = Param {
            name: "x".to_string(),
            type_annotation: None,
            default: None,
            is_variadic: false,
            is_keyword: false,
        };
        let json = serde_json::to_string(&param).expect("serialize");
        // Should skip None fields and false booleans don't need to be skipped
        // (they still appear with serde default)
        assert!(json.contains("\"name\":\"x\""));
        assert!(!json.contains("type_annotation"));
        assert!(!json.contains("default"));
    }

    #[test]
    fn test_location_serialize() {
        let loc = Location {
            file: PathBuf::from("src/lib.py"),
            line: 42,
            column: Some(0),
        };
        let json = serde_json::to_string(&loc).expect("serialize");
        assert!(json.contains("\"line\":42"));

        let loc_no_col = Location {
            file: PathBuf::from("src/lib.py"),
            line: 42,
            column: None,
        };
        let json = serde_json::to_string(&loc_no_col).expect("serialize");
        assert!(!json.contains("column"));
    }

    #[test]
    fn test_signature_with_generator() {
        let sig = Signature {
            params: vec![],
            return_type: Some("Iterator[int]".to_string()),
            is_async: false,
            is_generator: true,
        };
        let json = serde_json::to_string(&sig).expect("serialize");
        let deser: Signature = serde_json::from_str(&json).expect("deserialize");
        assert!(deser.is_generator);
        assert!(!deser.is_async);
    }

    #[test]
    fn test_variadic_and_keyword_params() {
        let params = vec![
            Param {
                name: "args".to_string(),
                type_annotation: None,
                default: None,
                is_variadic: true,
                is_keyword: false,
            },
            Param {
                name: "kwargs".to_string(),
                type_annotation: None,
                default: None,
                is_variadic: false,
                is_keyword: true,
            },
        ];
        let json = serde_json::to_string(&params).expect("serialize");
        let deser: Vec<Param> = serde_json::from_str(&json).expect("deserialize");
        assert!(deser[0].is_variadic);
        assert!(!deser[0].is_keyword);
        assert!(!deser[1].is_variadic);
        assert!(deser[1].is_keyword);
    }
}
