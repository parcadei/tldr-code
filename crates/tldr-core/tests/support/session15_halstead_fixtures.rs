use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Simple expression for Halstead metrics:
/// Operators: =, +, *, return (4 distinct, 4 total)
/// Operands: a, b, c, result, 2 (5 distinct, 5 total)
pub const PYTHON_HALSTEAD_SIMPLE: &str = r#"
def simple_math(a, b):
    result = a + b * 2
    return result
"#;

/// Empty function for Halstead edge case
pub const PYTHON_HALSTEAD_EMPTY: &str = r#"
def empty_function():
    pass
"#;

/// Function with high Halstead complexity
pub const PYTHON_HALSTEAD_COMPLEX: &str = r#"
def complex_calculation(x, y, z, w):
    a = x + y - z * w
    b = a / x + y ** 2
    c = (a + b) * (x - y) / (z + w)
    d = a if b > c else c
    result = a + b + c + d
    return result
"#;

pub fn create_temp_file(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("Failed to create parent dirs");
    }
    let mut file = fs::File::create(&path).expect("Failed to create temp file");
    file.write_all(content.as_bytes())
        .expect("Failed to write content");
    path
}
