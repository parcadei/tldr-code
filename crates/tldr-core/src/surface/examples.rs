//! Example usage string generation for API surface entries.
//!
//! Generates templated example strings from function signatures and type
//! annotations. For example:
//! - `json.loads(s: str)` -> `result = json.loads("example")`
//! - `Flask(__name__)` -> `app = Flask(__name__)`
//!
//! Example args are inferred from type annotations:
//! - `str` -> `"example"`
//! - `int` -> `42`
//! - `bool` -> `True`
//! - `list` -> `[]`
//! - `dict` -> `{}`
//! - `Path` -> `Path("file.txt")`
//! - `Optional[X]` -> `None`
//! - Unknown -> `...`

use super::types::{ApiKind, Param};

/// Generate an example usage string for a function or method.
///
/// # Arguments
/// * `module` - Module path (e.g., "json", "flask.app")
/// * `name` - Function/method name (e.g., "loads", "route")
/// * `kind` - The API kind (Function, Method, Class, etc.)
/// * `params` - Parameter list with type information
/// * `class_name` - Parent class name, if this is a method
///
/// # Returns
/// An example usage string, or None if no meaningful example can be generated.
pub fn generate_example(
    module: &str,
    name: &str,
    kind: ApiKind,
    params: &[Param],
    class_name: Option<&str>,
) -> Option<String> {
    match kind {
        ApiKind::Class => generate_class_example(module, name, params),
        ApiKind::Function => generate_function_example(module, name, params),
        ApiKind::Method | ApiKind::ClassMethod | ApiKind::StaticMethod => {
            generate_method_example(module, name, kind, params, class_name)
        }
        ApiKind::Property => generate_property_example(module, name, class_name),
        ApiKind::Constant => Some(format!("{}.{}", module, name)),
        _ => None,
    }
}

/// Generate example for a class constructor.
///
/// Template: `<var> = <module>.<Class>(<example_args>)`
fn generate_class_example(module: &str, class_name: &str, params: &[Param]) -> Option<String> {
    let var = conventional_var_name(class_name);
    let args = format_example_args(params, true);
    Some(format!("{var} = {module}.{class_name}({args})"))
}

/// Generate example for a top-level function.
///
/// Template: `result = <module>.<func>(<example_args>)`
fn generate_function_example(module: &str, func_name: &str, params: &[Param]) -> Option<String> {
    let args = format_example_args(params, false);
    Some(format!("result = {module}.{func_name}({args})"))
}

/// Generate example for a method call.
///
/// Template: `result = <var>.<method>(<example_args>)`
fn generate_method_example(
    _module: &str,
    method_name: &str,
    kind: ApiKind,
    params: &[Param],
    class_name: Option<&str>,
) -> Option<String> {
    let class = class_name.unwrap_or("obj");
    let var = conventional_var_name(class);

    // Skip 'self'/'cls' from params for method examples
    let skip_self = matches!(kind, ApiKind::Method | ApiKind::ClassMethod);
    let args = format_example_args(params, skip_self);

    match kind {
        ApiKind::StaticMethod => Some(format!("result = {class}.{method_name}({args})")),
        ApiKind::ClassMethod => Some(format!("result = {class}.{method_name}({args})")),
        _ => Some(format!("result = {var}.{method_name}({args})")),
    }
}

/// Generate example for a property access.
///
/// Template: `value = <var>.<property>`
fn generate_property_example(
    _module: &str,
    prop_name: &str,
    class_name: Option<&str>,
) -> Option<String> {
    let class = class_name.unwrap_or("obj");
    let var = conventional_var_name(class);
    Some(format!("value = {var}.{prop_name}"))
}

/// Generate a conventional variable name from a class name.
///
/// Uses lowercase first letter and common abbreviations:
/// - `Flask` -> `app`
/// - `JSONEncoder` -> `encoder`
/// - `MyClass` -> `my_class`
fn conventional_var_name(class_name: &str) -> String {
    // Common conventional names
    match class_name {
        "Flask" => return "app".to_string(),
        "Blueprint" => return "bp".to_string(),
        "Application" | "App" => return "app".to_string(),
        "Session" => return "session".to_string(),
        "Connection" | "Conn" => return "conn".to_string(),
        "Database" | "DB" => return "db".to_string(),
        "Client" => return "client".to_string(),
        "Server" => return "server".to_string(),
        "Request" => return "req".to_string(),
        "Response" => return "resp".to_string(),
        _ => {}
    }

    // Default: lowercase first letter or snake_case the CamelCase
    if class_name.chars().all(|c| c.is_uppercase() || c == '_') {
        // ALL_CAPS -> lowercase
        return class_name.to_lowercase();
    }

    // Simple lowercase for short names
    if class_name.len() <= 4 {
        return class_name.to_lowercase();
    }

    // CamelCase to snake_case
    let chars: Vec<char> = class_name.chars().collect();
    let mut result = String::new();
    for i in 0..chars.len() {
        let c = chars[i];
        if c.is_uppercase() && i > 0 {
            let prev = chars[i - 1];
            let next_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();
            // Insert underscore when:
            // 1. Previous char is lowercase (MyClass -> my_class)
            // 2. Previous char is uppercase AND next char is lowercase (JSONEncoder -> json_encoder)
            if prev.is_lowercase() || (prev.is_uppercase() && next_lower) {
                result.push('_');
            }
        }
        result.push(c.to_lowercase().next().unwrap_or(c));
    }
    result
}

/// Format example arguments from parameter list.
///
/// Generates example values based on type annotations:
/// - `str` -> `"example"`
/// - `int` -> `42`
/// - `bool` -> `True`
/// - `float` -> `3.14`
/// - `list` -> `[]`
/// - `dict` -> `{}`
/// - `bytes` -> `b"data"`
/// - `Path` -> `Path("file.txt")`
/// - `Optional[X]` -> `None`
/// - Unknown -> `...`
fn format_example_args(params: &[Param], skip_self: bool) -> String {
    let filtered: Vec<&Param> = params
        .iter()
        .filter(|p| {
            if skip_self && (p.name == "self" || p.name == "cls") {
                return false;
            }
            true
        })
        .collect();

    filtered
        .iter()
        .map(|p| example_for_param(p))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate an example value for a single parameter based on its type annotation.
fn example_for_param(param: &Param) -> String {
    // If there's a default, use it
    if let Some(default) = &param.default {
        return default.clone();
    }

    // If variadic, show as unpacked
    if param.is_variadic {
        return "*args".to_string();
    }
    if param.is_keyword {
        return "**kwargs".to_string();
    }

    // Infer from type annotation
    if let Some(type_ann) = &param.type_annotation {
        example_for_type(type_ann)
    } else {
        // No type annotation: use the parameter name as a hint
        "...".to_string()
    }
}

/// Generate an example value for a given type annotation string.
pub fn example_for_type(type_ann: &str) -> String {
    let normalized = type_ann.trim();

    // Handle Optional[X] -> None
    if normalized.starts_with("Optional[") {
        return "None".to_string();
    }

    // Handle Union types -> use first variant
    if normalized.starts_with("Union[") {
        if let Some(inner) = normalized
            .strip_prefix("Union[")
            .and_then(|s| s.strip_suffix(']'))
        {
            if let Some(first) = inner.split(',').next() {
                return example_for_type(first.trim());
            }
        }
        return "...".to_string();
    }

    // Handle List[X] / list[X]
    if normalized.starts_with("List[") || normalized.starts_with("list[") {
        return "[]".to_string();
    }

    // Handle Dict[K, V] / dict[K, V]
    if normalized.starts_with("Dict[") || normalized.starts_with("dict[") {
        return "{}".to_string();
    }

    // Handle Tuple[X, ...] / tuple[X, ...]
    if normalized.starts_with("Tuple[") || normalized.starts_with("tuple[") {
        return "()".to_string();
    }

    // Handle Set[X] / set[X]
    if normalized.starts_with("Set[") || normalized.starts_with("set[") {
        return "set()".to_string();
    }

    // Handle Callable
    if normalized.starts_with("Callable") {
        return "lambda: None".to_string();
    }

    // Simple types
    match normalized {
        "str" | "String" => "\"example\"".to_string(),
        "int" | "i32" | "i64" | "u32" | "u64" | "usize" | "isize" => "42".to_string(),
        "float" | "f32" | "f64" => "3.14".to_string(),
        "bool" => "True".to_string(),
        "bytes" | "bytearray" => "b\"data\"".to_string(),
        "None" | "NoneType" => "None".to_string(),
        "list" => "[]".to_string(),
        "dict" => "{}".to_string(),
        "tuple" => "()".to_string(),
        "set" | "frozenset" => "set()".to_string(),
        "Path" | "PathBuf" | "PurePath" | "PurePosixPath" => "Path(\"file.txt\")".to_string(),
        "Any" => "...".to_string(),
        "object" => "object()".to_string(),
        "type" => "object".to_string(),
        _ => "...".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::types::{ApiKind, Param};

    #[test]
    fn test_example_for_type_str() {
        assert_eq!(example_for_type("str"), "\"example\"");
    }

    #[test]
    fn test_example_for_type_int() {
        assert_eq!(example_for_type("int"), "42");
    }

    #[test]
    fn test_example_for_type_bool() {
        assert_eq!(example_for_type("bool"), "True");
    }

    #[test]
    fn test_example_for_type_float() {
        assert_eq!(example_for_type("float"), "3.14");
    }

    #[test]
    fn test_example_for_type_optional() {
        assert_eq!(example_for_type("Optional[str]"), "None");
    }

    #[test]
    fn test_example_for_type_list() {
        assert_eq!(example_for_type("List[int]"), "[]");
        assert_eq!(example_for_type("list[str]"), "[]");
        assert_eq!(example_for_type("list"), "[]");
    }

    #[test]
    fn test_example_for_type_dict() {
        assert_eq!(example_for_type("Dict[str, int]"), "{}");
        assert_eq!(example_for_type("dict"), "{}");
    }

    #[test]
    fn test_example_for_type_path() {
        assert_eq!(example_for_type("Path"), "Path(\"file.txt\")");
    }

    #[test]
    fn test_example_for_type_bytes() {
        assert_eq!(example_for_type("bytes"), "b\"data\"");
    }

    #[test]
    fn test_example_for_type_union() {
        assert_eq!(example_for_type("Union[str, int]"), "\"example\"");
    }

    #[test]
    fn test_example_for_type_callable() {
        assert_eq!(example_for_type("Callable"), "lambda: None");
    }

    #[test]
    fn test_example_for_type_unknown() {
        assert_eq!(example_for_type("MyCustomType"), "...");
    }

    #[test]
    fn test_conventional_var_name() {
        assert_eq!(conventional_var_name("Flask"), "app");
        assert_eq!(conventional_var_name("Client"), "client");
        assert_eq!(conventional_var_name("JSONEncoder"), "json_encoder");
        assert_eq!(conventional_var_name("MyClass"), "my_class");
    }

    #[test]
    fn test_generate_class_example() {
        let params = vec![Param {
            name: "name".to_string(),
            type_annotation: Some("str".to_string()),
            default: None,
            is_variadic: false,
            is_keyword: false,
        }];
        let example = generate_example("flask", "Flask", ApiKind::Class, &params, None);
        assert_eq!(example, Some("app = flask.Flask(\"example\")".to_string()));
    }

    #[test]
    fn test_generate_function_example() {
        let params = vec![Param {
            name: "s".to_string(),
            type_annotation: Some("str".to_string()),
            default: None,
            is_variadic: false,
            is_keyword: false,
        }];
        let example = generate_example("json", "loads", ApiKind::Function, &params, None);
        assert_eq!(
            example,
            Some("result = json.loads(\"example\")".to_string())
        );
    }

    #[test]
    fn test_generate_method_example() {
        let params = vec![
            Param {
                name: "self".to_string(),
                type_annotation: None,
                default: None,
                is_variadic: false,
                is_keyword: false,
            },
            Param {
                name: "key".to_string(),
                type_annotation: Some("str".to_string()),
                default: None,
                is_variadic: false,
                is_keyword: false,
            },
        ];
        let example = generate_example(
            "json",
            "encode",
            ApiKind::Method,
            &params,
            Some("JSONEncoder"),
        );
        assert_eq!(
            example,
            Some("result = json_encoder.encode(\"example\")".to_string())
        );
    }

    #[test]
    fn test_generate_static_method_example() {
        let params = vec![Param {
            name: "data".to_string(),
            type_annotation: Some("dict".to_string()),
            default: None,
            is_variadic: false,
            is_keyword: false,
        }];
        let example = generate_example(
            "my_module",
            "from_dict",
            ApiKind::StaticMethod,
            &params,
            Some("MyClass"),
        );
        assert_eq!(example, Some("result = MyClass.from_dict({})".to_string()));
    }

    #[test]
    fn test_generate_property_example() {
        let example = generate_example("flask", "url_map", ApiKind::Property, &[], Some("Flask"));
        assert_eq!(example, Some("value = app.url_map".to_string()));
    }

    #[test]
    fn test_generate_constant_example() {
        let example = generate_example("json", "HIGHEST_PROTOCOL", ApiKind::Constant, &[], None);
        assert_eq!(example, Some("json.HIGHEST_PROTOCOL".to_string()));
    }

    #[test]
    fn test_format_example_args_with_defaults() {
        let params = vec![Param {
            name: "indent".to_string(),
            type_annotation: Some("int".to_string()),
            default: Some("4".to_string()),
            is_variadic: false,
            is_keyword: false,
        }];
        let args = format_example_args(&params, false);
        assert_eq!(args, "4");
    }

    #[test]
    fn test_format_example_args_variadic() {
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
        let args = format_example_args(&params, false);
        assert_eq!(args, "*args, **kwargs");
    }

    #[test]
    fn test_format_example_args_skip_self() {
        let params = vec![
            Param {
                name: "self".to_string(),
                type_annotation: None,
                default: None,
                is_variadic: false,
                is_keyword: false,
            },
            Param {
                name: "key".to_string(),
                type_annotation: Some("str".to_string()),
                default: None,
                is_variadic: false,
                is_keyword: false,
            },
        ];
        let args = format_example_args(&params, true);
        assert_eq!(args, "\"example\"");
    }
}
