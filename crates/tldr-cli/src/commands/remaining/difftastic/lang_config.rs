use std::collections::HashSet;

/// Per-language configuration for tree-sitter to difftastic Syntax conversion.
pub struct LangConfig {
    /// Tree-sitter node kinds that should be flattened into a single Syntax::Atom.
    pub atom_nodes: HashSet<&'static str>,
    /// Pairs of opening/closing tokens that define Syntax::List delimiters.
    pub delimiter_tokens: Vec<(&'static str, &'static str)>,
}

/// All our 18 languages prefer inner delimiters (C-like syntax).
/// Difftastic's prefer_outer is only for Lisp/JSON/TOML/HCL/SQL which we don't support.
pub const PREFER_OUTER_DELIMITER: bool = false;

impl LangConfig {
    pub fn for_language(lang: &str) -> LangConfig {
        match lang {
            "python" => LangConfig {
                atom_nodes: ["string"].into_iter().collect(),
                delimiter_tokens: vec![("(", ")"), ("[", "]"), ("{", "}")],
            },
            "typescript" => LangConfig {
                atom_nodes: ["string", "template_string", "regex", "predefined_type"]
                    .into_iter()
                    .collect(),
                delimiter_tokens: vec![("{", "}"), ("(", ")"), ("[", "]"), ("<", ">")],
            },
            "javascript" => LangConfig {
                atom_nodes: ["string", "template_string", "regex"].into_iter().collect(),
                delimiter_tokens: vec![("[", "]"), ("(", ")"), ("{", "}"), ("<", ">")],
            },
            "go" => LangConfig {
                atom_nodes: ["interpreted_string_literal", "raw_string_literal"]
                    .into_iter()
                    .collect(),
                delimiter_tokens: vec![("{", "}"), ("[", "]"), ("(", ")")],
            },
            "rust" => LangConfig {
                atom_nodes: ["char_literal", "string_literal", "raw_string_literal"]
                    .into_iter()
                    .collect(),
                delimiter_tokens: vec![("{", "}"), ("(", ")"), ("[", "]"), ("|", "|"), ("<", ">")],
            },
            "java" => LangConfig {
                atom_nodes: [
                    "string_literal",
                    "boolean_type",
                    "integral_type",
                    "floating_point_type",
                    "void_type",
                ]
                .into_iter()
                .collect(),
                delimiter_tokens: vec![("(", ")"), ("{", "}"), ("[", "]")],
            },
            "c" => LangConfig {
                atom_nodes: ["string_literal", "char_literal"].into_iter().collect(),
                delimiter_tokens: vec![("(", ")"), ("{", "}"), ("[", "]")],
            },
            "cpp" | "c++" => LangConfig {
                atom_nodes: ["string_literal", "char_literal"].into_iter().collect(),
                delimiter_tokens: vec![("(", ")"), ("{", "}"), ("[", "]"), ("<", ">")],
            },
            "ruby" => LangConfig {
                atom_nodes: ["string", "heredoc_body", "regex"].into_iter().collect(),
                delimiter_tokens: vec![
                    ("{", "}"),
                    ("(", ")"),
                    ("[", "]"),
                    ("|", "|"),
                    ("def", "end"),
                    ("begin", "end"),
                    ("class", "end"),
                ],
            },
            "kotlin" => LangConfig {
                atom_nodes: [
                    "nullable_type",
                    "string_literal",
                    "line_string_literal",
                    "character_literal",
                ]
                .into_iter()
                .collect(),
                delimiter_tokens: vec![("(", ")"), ("{", "}"), ("[", "]"), ("<", ">")],
            },
            "swift" => LangConfig {
                atom_nodes: ["line_string_literal"].into_iter().collect(),
                delimiter_tokens: vec![("{", "}"), ("(", ")"), ("[", "]"), ("<", ">")],
            },
            "csharp" | "c#" => LangConfig {
                atom_nodes: [
                    "string_literal",
                    "verbatim_string_literal",
                    "character_literal",
                    "modifier",
                ]
                .into_iter()
                .collect(),
                delimiter_tokens: vec![("{", "}"), ("(", ")")],
            },
            "scala" => LangConfig {
                atom_nodes: ["string", "template_string"].into_iter().collect(),
                delimiter_tokens: vec![("{", "}"), ("(", ")"), ("[", "]")],
            },
            "php" => LangConfig {
                atom_nodes: ["string", "encapsed_string"].into_iter().collect(),
                delimiter_tokens: vec![("(", ")"), ("[", "]"), ("{", "}")],
            },
            "lua" => LangConfig {
                atom_nodes: ["string"].into_iter().collect(),
                delimiter_tokens: vec![("(", ")"), ("{", "}"), ("[", "]")],
            },
            "luau" => LangConfig {
                atom_nodes: ["string"].into_iter().collect(),
                delimiter_tokens: vec![("(", ")"), ("{", "}"), ("[", "]")],
            },
            "elixir" => LangConfig {
                atom_nodes: ["string", "sigil", "heredoc"].into_iter().collect(),
                delimiter_tokens: vec![("(", ")"), ("{", "}"), ("do", "end")],
            },
            "ocaml" => LangConfig {
                atom_nodes: [
                    "character",
                    "string",
                    "quoted_string",
                    "tag",
                    "type_variable",
                    "attribute_id",
                ]
                .into_iter()
                .collect(),
                delimiter_tokens: vec![("(", ")"), ("[", "]"), ("{", "}")],
            },
            // Fallback: no special atoms, standard delimiters
            _ => LangConfig {
                atom_nodes: HashSet::new(),
                delimiter_tokens: vec![("(", ")"), ("[", "]"), ("{", "}")],
            },
        }
    }
}
