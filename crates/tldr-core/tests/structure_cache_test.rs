use std::path::PathBuf;
use tempfile::TempDir;
use tldr_core::{get_code_structure, Language};
use tldr_core::{read_structure_cache, write_structure_cache};

#[test]
fn test_write_read_structure_cache_roundtrip() {
    let dir = TempDir::new().unwrap();

    // Create a Python project
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("main.py"),
        "def hello():\n    pass\n\ndef world():\n    pass\n",
    )
    .unwrap();

    // Build structure
    let structure = get_code_structure(&src, Language::Python, 0, None).unwrap();
    assert!(
        !structure.files[0].definitions.is_empty(),
        "Definitions should be populated (Phase 2)"
    );

    // Write cache
    let cache_path = dir.path().join("structure.json");
    write_structure_cache(&structure, &cache_path).unwrap();
    assert!(cache_path.exists(), "Cache file should exist");

    // Read cache
    let lookup = read_structure_cache(&cache_path).unwrap();

    // Verify: definitions for our file should be in the lookup
    let file_path = PathBuf::from("main.py");
    let defs = lookup
        .by_file
        .get(&file_path)
        .expect("Should find main.py in lookup");
    assert!(defs.len() >= 2, "Should have at least 2 definitions");
    assert!(defs.iter().any(|d| d.name == "hello"), "Should find hello");
    assert!(defs.iter().any(|d| d.name == "world"), "Should find world");
}

#[test]
fn test_read_structure_cache_missing_file_errors() {
    let result = read_structure_cache(std::path::Path::new("/nonexistent/structure.json"));
    assert!(result.is_err(), "Reading nonexistent cache should error");
}

#[test]
fn test_read_structure_cache_invalid_json_errors() {
    let dir = TempDir::new().unwrap();
    let cache_path = dir.path().join("bad.json");
    std::fs::write(&cache_path, "not valid json!!!").unwrap();
    let result = read_structure_cache(&cache_path);
    assert!(result.is_err(), "Reading invalid JSON should error");
}

#[test]
fn test_structure_lookup_finds_definitions_by_path() {
    let dir = TempDir::new().unwrap();

    // Create 2 Python files
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.py"), "def alpha():\n    pass\n").unwrap();
    std::fs::write(src.join("b.py"), "def beta():\n    pass\n").unwrap();

    let structure = get_code_structure(&src, Language::Python, 0, None).unwrap();
    let cache_path = dir.path().join("cache.json");
    write_structure_cache(&structure, &cache_path).unwrap();
    let lookup = read_structure_cache(&cache_path).unwrap();

    // Verify both files are in the lookup
    let a_defs = lookup
        .by_file
        .get(&PathBuf::from("a.py"))
        .expect("Should find a.py");
    assert!(a_defs.iter().any(|d| d.name == "alpha"));

    let b_defs = lookup
        .by_file
        .get(&PathBuf::from("b.py"))
        .expect("Should find b.py");
    assert!(b_defs.iter().any(|d| d.name == "beta"));

    // Missing file should return None
    assert!(!lookup.by_file.contains_key(&PathBuf::from("c.py")));
}
