//! Taint Analysis Tests
//!
//! Comprehensive test suite for CFG-based taint analysis as defined in
//! session11-taint-spec.md. These tests define expected behavior for:
//!
//! 1. Type Definition Tests - Enum variants and struct fields
//! 2. Pattern Matching Tests - Source/sink/sanitizer detection
//! 3. Worklist Algorithm Tests - Taint propagation
//! 4. Vulnerability Detection Tests - Source to sink flows
//! 5. Edge Case Tests - Empty functions, no sources, etc.
//!
//! All tests are marked #[ignore] as the taint module is not yet implemented.
//! Reference: session11-taint-spec.md Section 8

use std::collections::{HashMap, HashSet};

// Phase 1: Type imports (enabled)
use super::taint::{
    SanitizerType, TaintInfo, TaintSink, TaintSinkType, TaintSourceType,
};

// Phase 3: Pattern matching function imports (implemented)
use super::taint::{
    detect_sanitizer, detect_sinks, detect_sources, find_sanitizers_in_statement,
    find_sinks_in_statement, find_sources_in_statement, is_sanitizer,
};

// Phase 4: Worklist algorithm (implemented)
use super::taint::compute_taint;

use crate::types::{BlockType, CfgBlock, CfgEdge, CfgInfo, EdgeType, RefType, VarRef};
use crate::Language;

// =============================================================================
// Test Fixtures
// =============================================================================

mod fixtures {
    use super::*;

    /// Create a basic block with given id and line range
    pub fn make_block(id: usize, start: u32, end: u32) -> CfgBlock {
        CfgBlock {
            id,
            block_type: BlockType::Body,
            lines: (start, end),
            calls: Vec::new(),
        }
    }

    /// Create a definition VarRef
    pub fn make_def(name: &str, line: u32) -> VarRef {
        VarRef {
            name: name.to_string(),
            ref_type: RefType::Definition,
            line,
            column: 0,
            context: None,
            group_id: None,
        }
    }

    /// Create a use VarRef
    pub fn make_use(name: &str, line: u32) -> VarRef {
        VarRef {
            name: name.to_string(),
            ref_type: RefType::Use,
            line,
            column: 0,
            context: None,
            group_id: None,
        }
    }

    /// Linear CFG: Block 0 -> Block 1 -> Block 2
    /// Used for simple taint propagation tests
    pub fn linear_cfg() -> CfgInfo {
        CfgInfo {
            function: "linear".to_string(),
            blocks: vec![
                make_block(0, 1, 2),
                make_block(1, 3, 4),
                make_block(2, 5, 6),
            ],
            edges: vec![
                CfgEdge {
                    from: 0,
                    to: 1,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 1,
                    to: 2,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
            ],
            entry_block: 0,
            exit_blocks: vec![2],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        }
    }

    /// Loop CFG for convergence tests:
    /// Block 0 -> Block 1 (loop header) -> Block 2 (body) -back-> Block 1
    ///                                  -> Block 3 (exit)
    pub fn loop_cfg() -> CfgInfo {
        CfgInfo {
            function: "loop".to_string(),
            blocks: vec![
                make_block(0, 1, 2), // entry
                make_block(1, 3, 4), // loop header
                make_block(2, 5, 6), // loop body
                make_block(3, 7, 8), // exit
            ],
            edges: vec![
                CfgEdge {
                    from: 0,
                    to: 1,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 1,
                    to: 2,
                    edge_type: EdgeType::True,
                    condition: Some("i < n".to_string()),
                },
                CfgEdge {
                    from: 1,
                    to: 3,
                    edge_type: EdgeType::False,
                    condition: Some("i < n".to_string()),
                },
                CfgEdge {
                    from: 2,
                    to: 1,
                    edge_type: EdgeType::BackEdge,
                    condition: None,
                },
            ],
            entry_block: 0,
            exit_blocks: vec![3],
            cyclomatic_complexity: 2,
            nested_functions: HashMap::new(),
        }
    }

    /// Empty CFG for edge case tests
    pub fn empty_cfg() -> CfgInfo {
        CfgInfo {
            function: "empty".to_string(),
            blocks: vec![make_block(0, 1, 1)],
            edges: vec![],
            entry_block: 0,
            exit_blocks: vec![0],
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        }
    }
}

// =============================================================================
// Section 1: Type Definition Tests
// =============================================================================

/// Tests that TaintSourceType enum has all required variants
#[test]
fn test_taint_source_type_variants() {
    // TaintSourceType should have these variants per spec Section 1.2:
    // - UserInput: input(), sys.stdin.read()
    // - Stdin: sys.stdin.read(), sys.stdin.readline()
    // - HttpParam: request.args, request.form, request.values
    // - HttpBody: request.json, request.data, request.body
    // - EnvVar: os.environ, os.getenv()
    // - FileRead: optional, context-dependent

    let variants = [TaintSourceType::UserInput,
        TaintSourceType::Stdin,
        TaintSourceType::HttpParam,
        TaintSourceType::HttpBody,
        TaintSourceType::EnvVar,
        TaintSourceType::FileRead];
    assert_eq!(variants.len(), 6);
}

/// Tests that TaintSinkType enum has all required variants
#[test]
fn test_taint_sink_type_variants() {
    // TaintSinkType should have these variants per spec Section 1.3:
    // - SqlQuery: cursor.execute(), .execute()
    // - CodeEval: eval()
    // - CodeExec: exec()
    // - CodeCompile: compile()
    // - ShellExec: os.system(), subprocess.run()
    // - FileWrite: open(..., 'w'), .write_text()

    let variants = [TaintSinkType::SqlQuery,
        TaintSinkType::CodeEval,
        TaintSinkType::CodeExec,
        TaintSinkType::CodeCompile,
        TaintSinkType::ShellExec,
        TaintSinkType::FileWrite];
    assert_eq!(variants.len(), 6);
}

/// Tests that TaintInfo struct has required fields
#[test]
fn test_taint_info_struct_fields() {
    // TaintInfo should have these fields per spec Section 1.1:
    // - tainted_vars: HashMap<usize, HashSet<String>>
    // - sources: Vec<TaintSource>
    // - sinks: Vec<TaintSink>
    // - flows: Vec<TaintFlow>
    // - sanitized_vars: HashSet<String>
    // - function_name: String

    let info = TaintInfo::new("test_func");
    assert_eq!(info.function_name, "test_func");
    assert!(info.tainted_vars.is_empty());
    assert!(info.sources.is_empty());
    assert!(info.sinks.is_empty());
    assert!(info.flows.is_empty());
    assert!(info.sanitized_vars.is_empty());
}

/// Tests TaintInfo::is_tainted method
#[test]
fn test_taint_info_is_tainted() {
    let mut info = TaintInfo::new("test");
    let mut block_taint = HashSet::new();
    block_taint.insert("user_input".to_string());
    info.tainted_vars.insert(0, block_taint);

    assert!(info.is_tainted(0, "user_input"));
    assert!(!info.is_tainted(0, "other_var"));
    assert!(!info.is_tainted(1, "user_input")); // block 1 doesn't exist
}

/// Tests TaintInfo::is_tainted returns false for nonexistent block
#[test]
fn test_taint_info_is_tainted_nonexistent_block() {
    let info = TaintInfo::new("test");
    assert!(!info.is_tainted(999, "any_var"));
}

/// Tests TaintInfo::get_vulnerabilities returns only tainted sinks
#[test]
fn test_taint_info_get_vulnerabilities() {
    let mut info = TaintInfo::new("test");

    // Add a tainted sink (vulnerability)
    info.sinks.push(TaintSink {
        var: "query".to_string(),
        line: 5,
        sink_type: TaintSinkType::SqlQuery,
        tainted: true,
        statement: Some("cursor.execute(query)".to_string()),
    });

    // Add a non-tainted sink (safe)
    info.sinks.push(TaintSink {
        var: "safe_query".to_string(),
        line: 10,
        sink_type: TaintSinkType::SqlQuery,
        tainted: false,
        statement: Some("cursor.execute(safe_query)".to_string()),
    });

    let vulns = info.get_vulnerabilities();
    assert_eq!(vulns.len(), 1);
    assert_eq!(vulns[0].var, "query");
}

/// Tests TaintInfo default values
#[test]
fn test_taint_info_default_values() {
    let info = TaintInfo::default();
    assert!(info.function_name.is_empty());
    assert!(info.tainted_vars.is_empty());
    assert!(info.sources.is_empty());
    assert!(info.sinks.is_empty());
    assert!(info.flows.is_empty());
    assert!(info.sanitized_vars.is_empty());
}

// =============================================================================
// Section 2: Pattern Matching Tests - Source Detection
// =============================================================================

/// Tests detection of input() as source
#[test]
fn test_detect_input_as_source() {
    let stmt = "user_input = input()";
    let sources = find_sources_in_statement(stmt, 1, Language::Python);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].var, "user_input");
    assert!(matches!(sources[0].source_type, TaintSourceType::UserInput));
}

/// Tests detection of os.environ as source
#[test]
fn test_detect_os_environ_as_source() {
    let stmt = "value = os.environ['SECRET']";
    let sources = find_sources_in_statement(stmt, 1, Language::Python);
    assert_eq!(sources.len(), 1);
    assert!(matches!(sources[0].source_type, TaintSourceType::EnvVar));
}

/// Tests detection of os.getenv() as source
#[test]
fn test_detect_os_getenv_as_source() {
    let stmt = "value = os.getenv('API_KEY')";
    let sources = find_sources_in_statement(stmt, 1, Language::Python);
    assert_eq!(sources.len(), 1);
    assert!(matches!(sources[0].source_type, TaintSourceType::EnvVar));
}

/// Tests detection of request.args as source
#[test]
fn test_detect_request_args_as_source() {
    let stmt = "user_id = request.args.get('id')";
    let sources = find_sources_in_statement(stmt, 1, Language::Python);
    assert_eq!(sources.len(), 1);
    assert!(matches!(sources[0].source_type, TaintSourceType::HttpParam));
}

/// Tests detection of request.form as source
#[test]
fn test_detect_request_form_as_source() {
    let stmt = "username = request.form['username']";
    let sources = find_sources_in_statement(stmt, 1, Language::Python);
    assert_eq!(sources.len(), 1);
    assert!(matches!(sources[0].source_type, TaintSourceType::HttpParam));
}

/// Tests detection of request.json as source
#[test]
fn test_detect_request_json_as_source() {
    // request.json without method call - matches HttpParam pattern for "json"
    let stmt = "data = request.json";
    let sources = find_sources_in_statement(stmt, 1, Language::Python);
    assert_eq!(sources.len(), 1);
    // Note: request.json matches the HttpParam pattern, not HttpBody
    // HttpBody is for request.get_json()
    assert!(matches!(sources[0].source_type, TaintSourceType::HttpParam));
}

/// Tests detection of sys.stdin.read() as source
#[test]
fn test_detect_sys_stdin_as_source() {
    let stmt = "data = sys.stdin.read()";
    let sources = find_sources_in_statement(stmt, 1, Language::Python);
    // sys.stdin matches Stdin pattern, .read() matches FileRead pattern
    // We expect at least the stdin source
    assert!(!sources.is_empty());
    let has_stdin = sources
        .iter()
        .any(|s| matches!(s.source_type, TaintSourceType::Stdin));
    assert!(has_stdin, "Should detect sys.stdin as Stdin source");
}

// =============================================================================
// Section 3: Pattern Matching Tests - Sink Detection
// =============================================================================

/// Tests detection of cursor.execute() as SQL sink
#[test]
fn test_detect_execute_as_sql_sink() {
    let stmt = "cursor.execute(query)";
    let sinks = find_sinks_in_statement(stmt, 5, Language::Python);
    assert_eq!(sinks.len(), 1);
    assert_eq!(sinks[0].var, "query");
    assert!(matches!(sinks[0].sink_type, TaintSinkType::SqlQuery));
}

/// Tests detection of eval() as code sink
#[test]
fn test_detect_eval_as_code_sink() {
    let stmt = "result = eval(user_code)";
    let sinks = find_sinks_in_statement(stmt, 10, Language::Python);
    assert_eq!(sinks.len(), 1);
    assert_eq!(sinks[0].var, "user_code");
    assert!(matches!(sinks[0].sink_type, TaintSinkType::CodeEval));
}

/// Tests detection of exec() as code sink
#[test]
fn test_detect_exec_as_code_sink() {
    let stmt = "exec(dynamic_code)";
    let sinks = find_sinks_in_statement(stmt, 10, Language::Python);
    assert_eq!(sinks.len(), 1);
    assert_eq!(sinks[0].var, "dynamic_code");
    assert!(matches!(sinks[0].sink_type, TaintSinkType::CodeExec));
}

/// Tests detection of subprocess.run() as shell sink
#[test]
fn test_detect_subprocess_as_shell_sink() {
    let stmt = "subprocess.run(cmd, shell=True)";
    let sinks = find_sinks_in_statement(stmt, 15, Language::Python);
    assert_eq!(sinks.len(), 1);
    assert_eq!(sinks[0].var, "cmd");
    assert!(matches!(sinks[0].sink_type, TaintSinkType::ShellExec));
}

/// Tests detection of os.system() as shell sink
#[test]
fn test_detect_os_system_as_shell_sink() {
    let stmt = "os.system(command)";
    let sinks = find_sinks_in_statement(stmt, 20, Language::Python);
    assert_eq!(sinks.len(), 1);
    assert_eq!(sinks[0].var, "command");
    assert!(matches!(sinks[0].sink_type, TaintSinkType::ShellExec));
}

/// Tests detection of .write() as file write sink
#[test]
fn test_detect_write_as_file_sink() {
    let stmt = "f.write(data)";
    let sinks = find_sinks_in_statement(stmt, 25, Language::Python);
    assert_eq!(sinks.len(), 1);
    assert_eq!(sinks[0].var, "data");
    assert!(matches!(sinks[0].sink_type, TaintSinkType::FileWrite));
}

// =============================================================================
// Section 4: Pattern Matching Tests - Sanitizer Detection
// =============================================================================

/// Tests detection of int() as numeric sanitizer
#[test]
fn test_int_sanitizes_sql_injection() {
    let stmt = "safe_id = int(user_id)";
    assert!(is_sanitizer(stmt, Language::Python));

    // Alternatively check that it marks variable as sanitized
    let sanitizers = find_sanitizers_in_statement(stmt, 5, Language::Python);
    assert_eq!(sanitizers.len(), 1);
    assert_eq!(sanitizers[0].0, "safe_id");
    assert!(matches!(sanitizers[0].1, SanitizerType::Numeric));
}

/// Tests detection of shlex.quote() as shell sanitizer
#[test]
fn test_shlex_quote_sanitizes_command_injection() {
    let stmt = "safe_cmd = shlex.quote(user_input)";
    assert!(is_sanitizer(stmt, Language::Python));

    let sanitizers = find_sanitizers_in_statement(stmt, 5, Language::Python);
    assert_eq!(sanitizers.len(), 1);
    assert_eq!(sanitizers[0].0, "safe_cmd");
    assert!(matches!(sanitizers[0].1, SanitizerType::Shell));
}

/// Tests detection of html.escape() as HTML sanitizer
#[test]
fn test_html_escape_sanitizes_xss() {
    let stmt = "safe_html = html.escape(user_content)";
    assert!(is_sanitizer(stmt, Language::Python));

    let sanitizers = find_sanitizers_in_statement(stmt, 5, Language::Python);
    assert_eq!(sanitizers.len(), 1);
    assert_eq!(sanitizers[0].0, "safe_html");
    assert!(matches!(sanitizers[0].1, SanitizerType::Html));
}

// =============================================================================
// Section 5: Worklist Algorithm Tests - Taint Propagation
// =============================================================================

/// Tests simple assignment propagates taint: x = input(); y = x -> y is tainted
#[test]
fn test_propagate_through_assignment() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: x = input()
        make_def("x", 1),
        // Block 1: y = x
        make_use("x", 3),
        make_def("y", 3),
        // Block 2: use y
        make_use("y", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "x = input()".to_string());
    statements.insert(3, "y = x".to_string());
    statements.insert(5, "print(y)".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    assert!(result.is_tainted(0, "x"), "x should be tainted at block 0");
    assert!(result.is_tainted(1, "x"), "x should be tainted at block 1");
    assert!(
        result.is_tainted(1, "y"),
        "y should be tainted at block 1 (via x)"
    );
    assert!(result.is_tainted(2, "y"), "y should be tainted at block 2");
}

/// Tests taint propagates through string concatenation
#[test]
fn test_propagate_through_concatenation() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: user = input()
        make_def("user", 1),
        // Block 1: query = "SELECT * FROM users WHERE name = '" + user + "'"
        make_use("user", 3),
        make_def("query", 3),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "user = input()".to_string());
    statements.insert(
        3,
        "query = \"SELECT * FROM users WHERE name = '\" + user + \"'\"".to_string(),
    );

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();
    assert!(
        result.is_tainted(1, "query"),
        "query should be tainted via concatenation"
    );
}

/// Tests taint propagates across CFG blocks
#[test]
fn test_propagate_across_blocks() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: x = input()
        make_def("x", 1),
        // Block 2: use x (skipping block 1)
        make_use("x", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "x = input()".to_string());
    statements.insert(5, "print(x)".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();
    assert!(
        result.is_tainted(2, "x"),
        "taint should propagate to block 2"
    );
}

/// Tests taint does NOT propagate backward (forward analysis only)
#[test]
fn test_taint_does_not_propagate_backward() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: use x (before definition)
        make_use("x", 1),
        // Block 2: x = input() (definition comes later)
        make_def("x", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "print(x)".to_string());
    statements.insert(5, "x = input()".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();
    // x at block 0 should NOT be tainted because the source is at block 2
    assert!(
        !result.is_tainted(0, "x"),
        "taint should not propagate backward"
    );
}

/// Tests taint propagates through loop iterations
#[test]
fn test_propagate_through_loop() {
    use fixtures::*;

    let cfg = loop_cfg();
    let refs = vec![
        // Block 0 (entry): data = input()
        make_def("data", 1),
        // Block 2 (loop body): result = process(data)
        make_use("data", 5),
        make_def("result", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "data = input()".to_string());
    statements.insert(5, "result = process(data)".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();
    assert!(
        result.is_tainted(2, "data"),
        "data should be tainted in loop body"
    );
    assert!(result.is_tainted(2, "result"), "result should be tainted");
}

/// Tests sanitizer removes taint
#[test]
fn test_sanitizer_removes_taint() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: x = input()
        make_def("x", 1),
        // Block 1: y = int(x)  <- sanitizer
        make_use("x", 3),
        make_def("y", 3),
        // Block 2: use y
        make_use("y", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "x = input()".to_string());
    statements.insert(3, "y = int(x)".to_string());
    statements.insert(5, "print(y)".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    assert!(result.is_tainted(0, "x"), "x should be tainted");
    assert!(result.is_tainted(1, "x"), "x should still be tainted");
    assert!(
        !result.is_tainted(1, "y"),
        "y should NOT be tainted (sanitized)"
    );
    assert!(
        result.sanitized_vars.contains("y"),
        "y should be in sanitized_vars"
    );
}

/// Tests multiple sources merge correctly
#[test]
fn test_multiple_sources_merge() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: x = input(); y = os.environ['KEY']
        make_def("x", 1),
        make_def("y", 2),
        // Block 1: z = x + y
        make_use("x", 3),
        make_use("y", 3),
        make_def("z", 3),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "x = input()".to_string());
    statements.insert(2, "y = os.environ['KEY']".to_string());
    statements.insert(3, "z = x + y".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    // Both x and y are sources, so z should be tainted
    assert!(
        result.is_tainted(1, "z"),
        "z should be tainted from multiple sources"
    );

    // Should have 2 sources
    assert_eq!(result.sources.len(), 2);
}

/// Tests worklist algorithm converges on loops (no infinite loop)
#[test]
fn test_convergence_with_cycles() {
    use fixtures::*;

    let cfg = loop_cfg();
    let refs = vec![
        // Block 0: x = input()
        make_def("x", 1),
        // Block 1 (header): condition uses x
        make_use("x", 3),
        // Block 2 (body): x = x + 1
        make_use("x", 5),
        make_def("x", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "x = input()".to_string());
    statements.insert(3, "while x < 10:".to_string());
    statements.insert(5, "x = x + 1".to_string());

    // Should complete without infinite loop
    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    // Verify it converged correctly
    assert!(result.is_tainted(1, "x"));
    assert!(result.is_tainted(2, "x"));
}

// =============================================================================
// Section 6: Vulnerability Detection Tests
// =============================================================================

/// Tests detection of SQL injection: tainted data flows to cursor.execute()
#[test]
fn test_detect_sql_injection() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: user_input = input()
        make_def("user_input", 1),
        // Block 1: query = "SELECT * FROM users WHERE id = " + user_input
        make_use("user_input", 3),
        make_def("query", 3),
        // Block 2: cursor.execute(query)
        make_use("query", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "user_input = input()".to_string());
    statements.insert(
        3,
        "query = \"SELECT * FROM users WHERE id = \" + user_input".to_string(),
    );
    statements.insert(5, "cursor.execute(query)".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    let vulns = result.get_vulnerabilities();
    assert_eq!(vulns.len(), 1, "Should detect 1 SQL injection");
    assert!(matches!(vulns[0].sink_type, TaintSinkType::SqlQuery));

    // Should also have a flow recorded
    assert_eq!(result.flows.len(), 1);
    assert_eq!(result.flows[0].source.var, "user_input");
    assert_eq!(result.flows[0].sink.var, "query");
}

/// Tests detection of command injection: tainted data flows to os.system()
#[test]
fn test_detect_command_injection() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: cmd = input()
        make_def("cmd", 1),
        // Block 2: os.system(cmd)
        make_use("cmd", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "cmd = input()".to_string());
    statements.insert(5, "os.system(cmd)".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    let vulns = result.get_vulnerabilities();
    assert_eq!(vulns.len(), 1, "Should detect 1 command injection");
    assert!(matches!(vulns[0].sink_type, TaintSinkType::ShellExec));
}

/// Tests detection of code injection: tainted data flows to eval()
#[test]
fn test_detect_code_injection() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: code = request.json['code']
        make_def("code", 1),
        // Block 2: eval(code)
        make_use("code", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "code = request.json['code']".to_string());
    statements.insert(5, "eval(code)".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    let vulns = result.get_vulnerabilities();
    assert_eq!(vulns.len(), 1, "Should detect 1 code injection");
    assert!(matches!(vulns[0].sink_type, TaintSinkType::CodeEval));
}

/// Tests NO vulnerability when data is sanitized before sink
#[test]
fn test_no_vulnerability_when_sanitized() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: user_id = input()
        make_def("user_id", 1),
        // Block 1: safe_id = int(user_id)  <- sanitizer
        make_use("user_id", 3),
        make_def("safe_id", 3),
        // Block 2: cursor.execute("SELECT * FROM users WHERE id = " + str(safe_id))
        make_use("safe_id", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "user_id = input()".to_string());
    statements.insert(3, "safe_id = int(user_id)".to_string());
    statements.insert(
        5,
        "cursor.execute(\"SELECT * FROM users WHERE id = \" + str(safe_id))".to_string(),
    );

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    let vulns = result.get_vulnerabilities();
    assert!(
        vulns.is_empty(),
        "Should NOT detect vulnerability (sanitized)"
    );
    assert!(result.sanitized_vars.contains("safe_id"));
}

/// Tests NO vulnerability when sink uses untainted data
#[test]
fn test_no_vulnerability_when_untainted() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: query = "SELECT * FROM users"  (not from user input)
        make_def("query", 1),
        // Block 2: cursor.execute(query)
        make_use("query", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "query = \"SELECT * FROM users\"".to_string());
    statements.insert(5, "cursor.execute(query)".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    let vulns = result.get_vulnerabilities();
    assert!(
        vulns.is_empty(),
        "Should NOT detect vulnerability (untainted)"
    );
    assert!(result.sources.is_empty(), "Should have no sources");
}

/// Tests detection of multiple vulnerabilities
#[test]
fn test_multiple_vulnerabilities() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: data = input()
        make_def("data", 1),
        // Block 1: cursor.execute(data)
        make_use("data", 3),
        // Block 2: os.system(data)
        make_use("data", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "data = input()".to_string());
    statements.insert(3, "cursor.execute(data)".to_string());
    statements.insert(5, "os.system(data)".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    let vulns = result.get_vulnerabilities();
    assert_eq!(vulns.len(), 2, "Should detect 2 vulnerabilities");
}

// =============================================================================
// Section 7: Edge Case Tests
// =============================================================================

/// Tests empty function produces valid (empty) TaintInfo
#[test]
fn test_empty_function() {
    use fixtures::*;

    let cfg = empty_cfg();
    let refs: Vec<VarRef> = vec![];

    let result = compute_taint(&cfg, &refs, &HashMap::new(), Language::Python).unwrap();

    assert_eq!(result.function_name, "empty");
    assert!(result.sources.is_empty());
    assert!(result.sinks.is_empty());
    assert!(result.flows.is_empty());
    assert!(result.get_vulnerabilities().is_empty());
}

/// Tests function with no sources produces no tainted vars
#[test]
fn test_no_sources_in_function() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: x = 42  (constant, not user input)
        make_def("x", 1),
        // Block 1: y = x
        make_use("x", 3),
        make_def("y", 3),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "x = 42".to_string());
    statements.insert(3, "y = x".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    assert!(result.sources.is_empty(), "Should have no sources");
    // All blocks should have empty taint sets
    for vars in result.tainted_vars.values() {
        assert!(vars.is_empty(), "No variables should be tainted");
    }
}

/// Tests function with no sinks produces no vulnerabilities
#[test]
fn test_no_sinks_in_function() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: x = input()  (source)
        make_def("x", 1),
        // Block 1: y = x  (no sink)
        make_use("x", 3),
        make_def("y", 3),
        // Block 2: print(y)  (not a sink)
        make_use("y", 5),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "x = input()".to_string());
    statements.insert(3, "y = x".to_string());
    statements.insert(5, "print(y)".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    assert!(!result.sources.is_empty(), "Should have sources");
    assert!(result.sinks.is_empty(), "Should have no sinks");
    assert!(
        result.get_vulnerabilities().is_empty(),
        "Should have no vulns"
    );
}

/// Tests taint analysis handles unreachable code gracefully
#[test]
fn test_unreachable_code() {
    use fixtures::*;

    // Create a CFG with an unreachable block (no predecessors except itself)
    let mut cfg = linear_cfg();
    cfg.blocks.push(make_block(3, 7, 8)); // Unreachable block
                                          // No edge TO block 3, so it's unreachable

    let refs = vec![
        // Block 0: x = input()
        make_def("x", 1),
        // Block 3 (unreachable): y = x
        make_use("x", 7),
        make_def("y", 7),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "x = input()".to_string());
    statements.insert(7, "y = x".to_string());

    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    // Block 3 should not have tainted variables (unreachable)
    assert!(!result.is_tainted(3, "y"));
}

/// Tests indirect taint through function call (conservative assumption)
#[test]
fn test_indirect_taint_through_function_call() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![
        // Block 0: x = input()
        make_def("x", 1),
        // Block 1: y = unknown_func(x)  <- conservative: might propagate taint
        make_use("x", 3),
        make_def("y", 3),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "x = input()".to_string());
    statements.insert(3, "y = unknown_func(x)".to_string());

    // Conservative analysis: if a tainted variable is used in a function call,
    // the result is considered tainted (unless it's a known sanitizer)
    let result = compute_taint(&cfg, &refs, &statements, Language::Python).unwrap();

    // Conservative: y might be tainted
    // The spec says to use conservative assumptions for unknown functions
    assert!(
        result.is_tainted(1, "y"),
        "Conservative: function result is tainted"
    );
}

// =============================================================================
// Section 8: JSON Serialization Tests
// =============================================================================

/// Tests TaintInfo serializes to expected JSON structure
#[test]
#[ignore = "Phase 7: TaintInfo::to_json_value not implemented"]
fn test_taint_info_to_json() {
    // Phase 7: JSON Serialization
    // let mut info = TaintInfo::new("test_func");
    // info.sources.push(TaintSource {
    //     var: "user_input".to_string(),
    //     line: 2,
    //     source_type: TaintSourceType::UserInput,
    //     statement: Some("user_input = input()".to_string()),
    // });
    //
    // let json = info.to_json_value();
    //
    // assert_eq!(json["function"], "test_func");
    // assert!(json["sources"].is_array());
    // assert_eq!(json["sources"][0]["var"], "user_input");
    // assert_eq!(json["sources"][0]["line"], 2);
    // assert_eq!(json["vulnerability_count"], 0);
    todo!("Implement TaintInfo::to_json_value");
}

/// Tests TaintSourceType serializes with snake_case
#[test]
fn test_taint_source_type_serialization() {
    let source_type = TaintSourceType::HttpParam;
    let json = serde_json::to_string(&source_type).unwrap();
    assert_eq!(json, "\"http_param\"");

    let source_type = TaintSourceType::UserInput;
    let json = serde_json::to_string(&source_type).unwrap();
    assert_eq!(json, "\"user_input\"");
}

/// Tests TaintSinkType serializes with snake_case
#[test]
fn test_taint_sink_type_serialization() {
    let sink_type = TaintSinkType::SqlQuery;
    let json = serde_json::to_string(&sink_type).unwrap();
    assert_eq!(json, "\"sql_query\"");

    let sink_type = TaintSinkType::ShellExec;
    let json = serde_json::to_string(&sink_type).unwrap();
    assert_eq!(json, "\"shell_exec\"");
}

// =============================================================================
// Section 10: Language-Specific Taint Pattern Tests (Phase 2 TDD)
//
// These tests define the expected behavior for taint pattern detection
// across all 18 supported languages. Each test is marked #[ignore] because
// the language-specific patterns have not yet been implemented (Phase 3).
//
// Languages are grouped by similarity:
//   - TypeScript + JavaScript (share patterns)
//   - Lua + Luau (share patterns)
//   - All others have unique test groups
// =============================================================================

// =========================================================================
// TypeScript Taint Patterns
// =========================================================================

#[test]
fn test_typescript_detect_sources() {
    // req.body -> HttpBody
    let sources = detect_sources("const data = req.body.username", 1, Language::TypeScript);
    assert!(
        !sources.is_empty(),
        "TypeScript req.body should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpBody),
        "req.body should be HttpBody, got: {:?}",
        sources
    );

    // req.query -> HttpParam
    let sources = detect_sources("const q = req.query.search", 2, Language::TypeScript);
    assert!(
        !sources.is_empty(),
        "TypeScript req.query should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "req.query should be HttpParam, got: {:?}",
        sources
    );

    // req.params -> HttpParam
    let sources = detect_sources("const id = req.params.id", 3, Language::TypeScript);
    assert!(
        !sources.is_empty(),
        "TypeScript req.params should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "req.params should be HttpParam, got: {:?}",
        sources
    );

    // process.env -> EnvVar
    let sources = detect_sources("const key = process.env.API_KEY", 4, Language::TypeScript);
    assert!(
        !sources.is_empty(),
        "TypeScript process.env should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "process.env should be EnvVar, got: {:?}",
        sources
    );

    // process.stdin -> Stdin
    let sources = detect_sources(
        "const input = process.stdin.read()",
        5,
        Language::TypeScript,
    );
    assert!(
        !sources.is_empty(),
        "TypeScript process.stdin should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::Stdin),
        "process.stdin should be Stdin, got: {:?}",
        sources
    );
}

#[test]
fn test_typescript_detect_sinks() {
    // eval() -> CodeEval
    let sinks = detect_sinks("eval(userInput)", 1, Language::TypeScript);
    assert!(
        !sinks.is_empty(),
        "TypeScript eval() should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "eval should be CodeEval, got: {:?}",
        sinks
    );

    // new Function() -> CodeEval
    let sinks = detect_sinks("const fn = new Function(code)", 2, Language::TypeScript);
    assert!(
        !sinks.is_empty(),
        "TypeScript new Function() should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "new Function should be CodeEval, got: {:?}",
        sinks
    );

    // child_process.exec -> ShellExec
    let sinks = detect_sinks("child_process.exec(cmd)", 3, Language::TypeScript);
    assert!(
        !sinks.is_empty(),
        "TypeScript child_process.exec should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "child_process.exec should be ShellExec, got: {:?}",
        sinks
    );

    // execSync -> ShellExec
    let sinks = detect_sinks("execSync(cmd)", 4, Language::TypeScript);
    assert!(
        !sinks.is_empty(),
        "TypeScript execSync should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "execSync should be ShellExec, got: {:?}",
        sinks
    );

    // innerHTML -> CodeEval (XSS)
    let sinks = detect_sinks("element.innerHTML = userContent", 5, Language::TypeScript);
    assert!(
        !sinks.is_empty(),
        "TypeScript innerHTML assignment should be detected as sink"
    );

    // document.write -> CodeEval (XSS)
    let sinks = detect_sinks("document.write(html)", 6, Language::TypeScript);
    assert!(
        !sinks.is_empty(),
        "TypeScript document.write should be detected as sink"
    );

    // db.query -> SqlQuery
    let sinks = detect_sinks("db.query(sql)", 7, Language::TypeScript);
    assert!(
        !sinks.is_empty(),
        "TypeScript db.query should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::SqlQuery),
        "db.query should be SqlQuery, got: {:?}",
        sinks
    );
}

#[test]
fn test_typescript_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("parseInt(val)", Language::TypeScript),
        Some(SanitizerType::Numeric),
        "parseInt should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("Number(val)", Language::TypeScript),
        Some(SanitizerType::Numeric),
        "Number() should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("encodeURIComponent(val)", Language::TypeScript),
        Some(SanitizerType::Html),
        "encodeURIComponent should be Html sanitizer"
    );
    assert_eq!(
        detect_sanitizer("DOMPurify.sanitize(html)", Language::TypeScript),
        Some(SanitizerType::Html),
        "DOMPurify.sanitize should be Html sanitizer"
    );
}

// =========================================================================
// JavaScript Taint Patterns (shares patterns with TypeScript)
// =========================================================================

#[test]
fn test_javascript_detect_sources() {
    // req.body -> HttpBody
    let sources = detect_sources("var data = req.body.username", 1, Language::JavaScript);
    assert!(
        !sources.is_empty(),
        "JavaScript req.body should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpBody),
        "req.body should be HttpBody, got: {:?}",
        sources
    );

    // req.query -> HttpParam
    let sources = detect_sources("var q = req.query.search", 2, Language::JavaScript);
    assert!(
        !sources.is_empty(),
        "JavaScript req.query should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "req.query should be HttpParam, got: {:?}",
        sources
    );

    // process.env -> EnvVar
    let sources = detect_sources("var key = process.env.SECRET", 3, Language::JavaScript);
    assert!(
        !sources.is_empty(),
        "JavaScript process.env should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "process.env should be EnvVar, got: {:?}",
        sources
    );
}

#[test]
fn test_javascript_detect_sinks() {
    // eval() -> CodeEval
    let sinks = detect_sinks("eval(input)", 1, Language::JavaScript);
    assert!(
        !sinks.is_empty(),
        "JavaScript eval() should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "eval should be CodeEval, got: {:?}",
        sinks
    );

    // child_process.spawn -> ShellExec
    let sinks = detect_sinks("child_process.spawn(cmd, args)", 2, Language::JavaScript);
    assert!(
        !sinks.is_empty(),
        "JavaScript child_process.spawn should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "child_process.spawn should be ShellExec, got: {:?}",
        sinks
    );
}

#[test]
fn test_javascript_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("parseInt(val, 10)", Language::JavaScript),
        Some(SanitizerType::Numeric),
        "parseInt should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("encodeURIComponent(val)", Language::JavaScript),
        Some(SanitizerType::Html),
        "encodeURIComponent should be Html sanitizer"
    );
}

// =========================================================================
// Go Taint Patterns
// =========================================================================

#[test]
fn test_go_detect_sources() {
    // r.FormValue -> HttpParam
    let sources = detect_sources("name := r.FormValue(\"name\")", 1, Language::Go);
    assert!(
        !sources.is_empty(),
        "Go r.FormValue should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "r.FormValue should be HttpParam, got: {:?}",
        sources
    );

    // r.URL.Query() -> HttpParam
    let sources = detect_sources("query := r.URL.Query()", 2, Language::Go);
    assert!(
        !sources.is_empty(),
        "Go r.URL.Query() should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "r.URL.Query() should be HttpParam, got: {:?}",
        sources
    );

    // os.Getenv -> EnvVar
    let sources = detect_sources("secret := os.Getenv(\"SECRET\")", 3, Language::Go);
    assert!(
        !sources.is_empty(),
        "Go os.Getenv should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "os.Getenv should be EnvVar, got: {:?}",
        sources
    );

    // os.Stdin -> Stdin
    let sources = detect_sources("reader := bufio.NewReader(os.Stdin)", 4, Language::Go);
    assert!(
        !sources.is_empty(),
        "Go os.Stdin/bufio.NewReader should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::Stdin),
        "os.Stdin should be Stdin, got: {:?}",
        sources
    );

    // fmt.Scan -> UserInput
    let sources = detect_sources("fmt.Scan(&input)", 5, Language::Go);
    assert!(
        !sources.is_empty(),
        "Go fmt.Scan should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "fmt.Scan should be UserInput, got: {:?}",
        sources
    );
}

#[test]
fn test_go_detect_sinks() {
    // exec.Command -> ShellExec
    let sinks = detect_sinks("exec.Command(cmd, args...)", 1, Language::Go);
    assert!(
        !sinks.is_empty(),
        "Go exec.Command should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "exec.Command should be ShellExec, got: {:?}",
        sinks
    );

    // db.Exec -> SqlQuery
    let sinks = detect_sinks("db.Exec(query, args...)", 2, Language::Go);
    assert!(!sinks.is_empty(), "Go db.Exec should be detected as sink");
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::SqlQuery),
        "db.Exec should be SqlQuery, got: {:?}",
        sinks
    );

    // db.Query -> SqlQuery
    let sinks = detect_sinks("rows, err := db.Query(sql)", 3, Language::Go);
    assert!(!sinks.is_empty(), "Go db.Query should be detected as sink");
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::SqlQuery),
        "db.Query should be SqlQuery, got: {:?}",
        sinks
    );

    // template.HTML -> CodeEval (XSS via unescaped HTML)
    let sinks = detect_sinks("out := template.HTML(userInput)", 4, Language::Go);
    assert!(
        !sinks.is_empty(),
        "Go template.HTML should be detected as sink"
    );

    // fmt.Fprintf(w, ...) -> FileWrite (response body injection)
    let sinks = detect_sinks("fmt.Fprintf(w, userInput)", 5, Language::Go);
    assert!(
        !sinks.is_empty(),
        "Go fmt.Fprintf(w,...) should be detected as sink"
    );
}

#[test]
fn test_go_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("n, err := strconv.Atoi(val)", Language::Go),
        Some(SanitizerType::Numeric),
        "strconv.Atoi should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("n, err := strconv.ParseInt(val, 10, 64)", Language::Go),
        Some(SanitizerType::Numeric),
        "strconv.ParseInt should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("safe := html.EscapeString(val)", Language::Go),
        Some(SanitizerType::Html),
        "html.EscapeString should be Html sanitizer"
    );
    assert_eq!(
        detect_sanitizer("safe := url.QueryEscape(val)", Language::Go),
        Some(SanitizerType::Html),
        "url.QueryEscape should be Html sanitizer"
    );
}

// =========================================================================
// Java Taint Patterns
// =========================================================================

#[test]
fn test_java_detect_sources() {
    // Scanner(System.in) -> Stdin
    let sources = detect_sources("Scanner sc = new Scanner(System.in)", 1, Language::Java);
    assert!(
        !sources.is_empty(),
        "Java Scanner(System.in) should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::Stdin),
        "Scanner(System.in) should be Stdin, got: {:?}",
        sources
    );

    // request.getParameter -> HttpParam
    let sources = detect_sources(
        "String name = request.getParameter(\"name\")",
        2,
        Language::Java,
    );
    assert!(
        !sources.is_empty(),
        "Java request.getParameter should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "request.getParameter should be HttpParam, got: {:?}",
        sources
    );

    // System.getenv -> EnvVar
    let sources = detect_sources("String key = System.getenv(\"API_KEY\")", 3, Language::Java);
    assert!(
        !sources.is_empty(),
        "Java System.getenv should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "System.getenv should be EnvVar, got: {:?}",
        sources
    );

    // BufferedReader -> UserInput
    let sources = detect_sources(
        "BufferedReader br = new BufferedReader(new InputStreamReader(System.in))",
        4,
        Language::Java,
    );
    assert!(
        !sources.is_empty(),
        "Java BufferedReader should be detected as source"
    );

    // readLine() -> UserInput
    let sources = detect_sources("String line = br.readLine()", 5, Language::Java);
    assert!(
        !sources.is_empty(),
        "Java readLine() should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "readLine should be UserInput, got: {:?}",
        sources
    );
}

#[test]
fn test_java_detect_sinks() {
    // Runtime.getRuntime().exec -> ShellExec
    let sinks = detect_sinks("Runtime.getRuntime().exec(cmd)", 1, Language::Java);
    assert!(
        !sinks.is_empty(),
        "Java Runtime.exec should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "Runtime.exec should be ShellExec, got: {:?}",
        sinks
    );

    // ProcessBuilder -> ShellExec
    let sinks = detect_sinks("new ProcessBuilder(cmd).start()", 2, Language::Java);
    assert!(
        !sinks.is_empty(),
        "Java ProcessBuilder should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "ProcessBuilder should be ShellExec, got: {:?}",
        sinks
    );

    // Statement.execute -> SqlQuery
    let sinks = detect_sinks("stmt.execute(sql)", 3, Language::Java);
    assert!(
        !sinks.is_empty(),
        "Java Statement.execute should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::SqlQuery),
        "Statement.execute should be SqlQuery, got: {:?}",
        sinks
    );

    // Class.forName -> CodeEval (reflection)
    let sinks = detect_sinks("Class.forName(className)", 4, Language::Java);
    assert!(
        !sinks.is_empty(),
        "Java Class.forName should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "Class.forName should be CodeEval, got: {:?}",
        sinks
    );
}

#[test]
fn test_java_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("int id = Integer.parseInt(val)", Language::Java),
        Some(SanitizerType::Numeric),
        "Integer.parseInt should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("long id = Long.parseLong(val)", Language::Java),
        Some(SanitizerType::Numeric),
        "Long.parseLong should be Numeric sanitizer"
    );
}

// =========================================================================
// Rust Taint Patterns
// =========================================================================

#[test]
fn test_rust_detect_sources() {
    // std::io::stdin() -> Stdin
    let sources = detect_sources(
        "let mut input = String::new(); std::io::stdin().read_line(&mut input)",
        1,
        Language::Rust,
    );
    assert!(
        !sources.is_empty(),
        "Rust std::io::stdin() should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::Stdin),
        "std::io::stdin should be Stdin, got: {:?}",
        sources
    );

    // std::env::var -> EnvVar
    let sources = detect_sources(
        "let key = std::env::var(\"API_KEY\").unwrap()",
        2,
        Language::Rust,
    );
    assert!(
        !sources.is_empty(),
        "Rust std::env::var should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "std::env::var should be EnvVar, got: {:?}",
        sources
    );

    // std::env::args -> UserInput
    let sources = detect_sources(
        "let args: Vec<String> = std::env::args().collect()",
        3,
        Language::Rust,
    );
    assert!(
        !sources.is_empty(),
        "Rust std::env::args should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "std::env::args should be UserInput, got: {:?}",
        sources
    );

    // std::fs::read_to_string -> FileRead
    let sources = detect_sources(
        "let content = std::fs::read_to_string(path).unwrap()",
        4,
        Language::Rust,
    );
    assert!(
        !sources.is_empty(),
        "Rust std::fs::read_to_string should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::FileRead),
        "std::fs::read_to_string should be FileRead, got: {:?}",
        sources
    );
}

#[test]
fn test_rust_detect_sinks() {
    // Command::new -> ShellExec
    let sinks = detect_sinks("Command::new(cmd).arg(arg).spawn()", 1, Language::Rust);
    assert!(
        !sinks.is_empty(),
        "Rust Command::new should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "Command::new should be ShellExec, got: {:?}",
        sinks
    );

    // std::process::Command -> ShellExec
    let sinks = detect_sinks("std::process::Command::new(cmd)", 2, Language::Rust);
    assert!(
        !sinks.is_empty(),
        "Rust std::process::Command should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "std::process::Command should be ShellExec, got: {:?}",
        sinks
    );

    // unsafe block -> CodeExec (unsafe operations)
    let sinks = detect_sinks("unsafe { std::ptr::write(ptr, val) }", 3, Language::Rust);
    assert!(
        !sinks.is_empty(),
        "Rust unsafe block should be detected as sink"
    );
}

#[test]
fn test_rust_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("let n: i32 = val.parse::<i32>().unwrap()", Language::Rust),
        Some(SanitizerType::Numeric),
        ".parse::<i32>() should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("let f: f64 = val.parse::<f64>().unwrap()", Language::Rust),
        Some(SanitizerType::Numeric),
        ".parse::<f64>() should be Numeric sanitizer"
    );
}

// =========================================================================
// C Taint Patterns
// =========================================================================

#[test]
fn test_c_detect_sources() {
    // scanf -> UserInput
    let sources = detect_sources("scanf(\"%s\", buf)", 1, Language::C);
    assert!(!sources.is_empty(), "C scanf should be detected as source");
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "scanf should be UserInput, got: {:?}",
        sources
    );

    // fgets -> UserInput
    let sources = detect_sources("fgets(buf, sizeof(buf), stdin)", 2, Language::C);
    assert!(!sources.is_empty(), "C fgets should be detected as source");
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "fgets should be UserInput, got: {:?}",
        sources
    );

    // gets -> UserInput (dangerous, deprecated)
    let sources = detect_sources("gets(buf)", 3, Language::C);
    assert!(!sources.is_empty(), "C gets should be detected as source");
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "gets should be UserInput, got: {:?}",
        sources
    );

    // getenv -> EnvVar
    let sources = detect_sources("char *val = getenv(\"PATH\")", 4, Language::C);
    assert!(!sources.is_empty(), "C getenv should be detected as source");
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "getenv should be EnvVar, got: {:?}",
        sources
    );

    // recv -> UserInput (network)
    let sources = detect_sources("int n = recv(sockfd, buf, len, 0)", 5, Language::C);
    assert!(!sources.is_empty(), "C recv should be detected as source");
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "recv should be UserInput, got: {:?}",
        sources
    );

    // fread -> FileRead
    let sources = detect_sources("size_t n = fread(buf, 1, size, fp)", 6, Language::C);
    assert!(!sources.is_empty(), "C fread should be detected as source");
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::FileRead),
        "fread should be FileRead, got: {:?}",
        sources
    );
}

#[test]
fn test_c_detect_sinks() {
    // system() -> ShellExec
    let sinks = detect_sinks("system(cmd)", 1, Language::C);
    assert!(!sinks.is_empty(), "C system() should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "system should be ShellExec, got: {:?}",
        sinks
    );

    // popen -> ShellExec
    let sinks = detect_sinks("FILE *fp = popen(cmd, \"r\")", 2, Language::C);
    assert!(!sinks.is_empty(), "C popen should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "popen should be ShellExec, got: {:?}",
        sinks
    );

    // sprintf -> FileWrite (format string vulnerability)
    let sinks = detect_sinks("sprintf(buf, fmt, user_data)", 3, Language::C);
    assert!(!sinks.is_empty(), "C sprintf should be detected as sink");

    // strcpy -> FileWrite (buffer overflow)
    let sinks = detect_sinks("strcpy(dest, src)", 4, Language::C);
    assert!(!sinks.is_empty(), "C strcpy should be detected as sink");

    // strcat -> FileWrite (buffer overflow)
    let sinks = detect_sinks("strcat(dest, src)", 5, Language::C);
    assert!(!sinks.is_empty(), "C strcat should be detected as sink");

    // exec -> ShellExec
    let sinks = detect_sinks("execvp(cmd, argv)", 6, Language::C);
    assert!(!sinks.is_empty(), "C exec should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "exec should be ShellExec, got: {:?}",
        sinks
    );
}

#[test]
fn test_c_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("int n = atoi(val)", Language::C),
        Some(SanitizerType::Numeric),
        "atoi should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("long n = strtol(val, NULL, 10)", Language::C),
        Some(SanitizerType::Numeric),
        "strtol should be Numeric sanitizer"
    );
    // snprintf is a bounded sanitizer (prevents buffer overflow)
    assert_eq!(
        detect_sanitizer("snprintf(buf, sizeof(buf), \"%s\", val)", Language::C),
        Some(SanitizerType::Shell),
        "snprintf should be Shell sanitizer (bounded write)"
    );
}

// =========================================================================
// C++ Taint Patterns
// =========================================================================

#[test]
fn test_cpp_detect_sources() {
    // std::cin >> -> UserInput
    let sources = detect_sources("std::cin >> input", 1, Language::Cpp);
    assert!(
        !sources.is_empty(),
        "C++ std::cin >> should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "std::cin should be UserInput, got: {:?}",
        sources
    );

    // std::getline -> UserInput
    let sources = detect_sources("std::getline(std::cin, line)", 2, Language::Cpp);
    assert!(
        !sources.is_empty(),
        "C++ std::getline should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "std::getline should be UserInput, got: {:?}",
        sources
    );

    // getenv -> EnvVar
    let sources = detect_sources("char* val = getenv(\"PATH\")", 3, Language::Cpp);
    assert!(
        !sources.is_empty(),
        "C++ getenv should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "getenv should be EnvVar, got: {:?}",
        sources
    );

    // std::ifstream -> FileRead
    let sources = detect_sources("std::ifstream file(path)", 4, Language::Cpp);
    assert!(
        !sources.is_empty(),
        "C++ std::ifstream should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::FileRead),
        "std::ifstream should be FileRead, got: {:?}",
        sources
    );
}

#[test]
fn test_cpp_detect_sinks() {
    // system() -> ShellExec
    let sinks = detect_sinks("system(cmd.c_str())", 1, Language::Cpp);
    assert!(!sinks.is_empty(), "C++ system() should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "system should be ShellExec, got: {:?}",
        sinks
    );

    // std::system -> ShellExec
    let sinks = detect_sinks("std::system(cmd)", 2, Language::Cpp);
    assert!(
        !sinks.is_empty(),
        "C++ std::system should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "std::system should be ShellExec, got: {:?}",
        sinks
    );

    // popen -> ShellExec
    let sinks = detect_sinks("FILE* fp = popen(cmd, \"r\")", 3, Language::Cpp);
    assert!(!sinks.is_empty(), "C++ popen should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "popen should be ShellExec, got: {:?}",
        sinks
    );

    // sprintf -> buffer overflow
    let sinks = detect_sinks("sprintf(buf, \"%s\", input.c_str())", 4, Language::Cpp);
    assert!(!sinks.is_empty(), "C++ sprintf should be detected as sink");
}

#[test]
fn test_cpp_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("int n = std::stoi(val)", Language::Cpp),
        Some(SanitizerType::Numeric),
        "std::stoi should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("long n = std::stol(val)", Language::Cpp),
        Some(SanitizerType::Numeric),
        "std::stol should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("int n = static_cast<int>(val)", Language::Cpp),
        Some(SanitizerType::Numeric),
        "static_cast<int> should be Numeric sanitizer"
    );
}

// =========================================================================
// Ruby Taint Patterns
// =========================================================================

#[test]
fn test_ruby_detect_sources() {
    // gets -> UserInput
    let sources = detect_sources("input = gets.chomp", 1, Language::Ruby);
    assert!(
        !sources.is_empty(),
        "Ruby gets should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "gets should be UserInput, got: {:?}",
        sources
    );

    // STDIN.read -> Stdin
    let sources = detect_sources("data = STDIN.read", 2, Language::Ruby);
    assert!(
        !sources.is_empty(),
        "Ruby STDIN.read should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::Stdin),
        "STDIN.read should be Stdin, got: {:?}",
        sources
    );

    // params[] -> HttpParam (Rails)
    let sources = detect_sources("name = params[:name]", 3, Language::Ruby);
    assert!(
        !sources.is_empty(),
        "Ruby params[] should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "params should be HttpParam, got: {:?}",
        sources
    );

    // ENV[] -> EnvVar
    let sources = detect_sources("key = ENV['SECRET_KEY']", 4, Language::Ruby);
    assert!(
        !sources.is_empty(),
        "Ruby ENV[] should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "ENV should be EnvVar, got: {:?}",
        sources
    );

    // File.read -> FileRead
    let sources = detect_sources("content = File.read(path)", 5, Language::Ruby);
    assert!(
        !sources.is_empty(),
        "Ruby File.read should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::FileRead),
        "File.read should be FileRead, got: {:?}",
        sources
    );
}

#[test]
fn test_ruby_detect_sinks() {
    // eval -> CodeEval
    let sinks = detect_sinks("eval(code)", 1, Language::Ruby);
    assert!(!sinks.is_empty(), "Ruby eval should be detected as sink");
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "eval should be CodeEval, got: {:?}",
        sinks
    );

    // system -> ShellExec
    let sinks = detect_sinks("system(cmd)", 2, Language::Ruby);
    assert!(!sinks.is_empty(), "Ruby system should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "system should be ShellExec, got: {:?}",
        sinks
    );

    // exec -> ShellExec
    let sinks = detect_sinks("exec(cmd)", 3, Language::Ruby);
    assert!(!sinks.is_empty(), "Ruby exec should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "exec should be ShellExec, got: {:?}",
        sinks
    );

    // IO.popen -> ShellExec
    let sinks = detect_sinks("IO.popen(cmd)", 4, Language::Ruby);
    assert!(
        !sinks.is_empty(),
        "Ruby IO.popen should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "IO.popen should be ShellExec, got: {:?}",
        sinks
    );

    // send -> CodeEval (dynamic dispatch)
    let sinks = detect_sinks("obj.send(method_name)", 5, Language::Ruby);
    assert!(!sinks.is_empty(), "Ruby send should be detected as sink");
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "send should be CodeEval, got: {:?}",
        sinks
    );
}

#[test]
fn test_ruby_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("n = val.to_i", Language::Ruby),
        Some(SanitizerType::Numeric),
        ".to_i should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("n = val.to_f", Language::Ruby),
        Some(SanitizerType::Numeric),
        ".to_f should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("safe = CGI.escapeHTML(val)", Language::Ruby),
        Some(SanitizerType::Html),
        "CGI.escapeHTML should be Html sanitizer"
    );
    assert_eq!(
        detect_sanitizer("safe = Rack::Utils.escape_html(val)", Language::Ruby),
        Some(SanitizerType::Html),
        "Rack::Utils.escape_html should be Html sanitizer"
    );
}

// =========================================================================
// Kotlin Taint Patterns
// =========================================================================

#[test]
fn test_kotlin_detect_sources() {
    // readLine() -> UserInput
    let sources = detect_sources("val input = readLine()", 1, Language::Kotlin);
    assert!(
        !sources.is_empty(),
        "Kotlin readLine() should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "readLine should be UserInput, got: {:?}",
        sources
    );

    // readln() -> UserInput
    let sources = detect_sources("val input = readln()", 2, Language::Kotlin);
    assert!(
        !sources.is_empty(),
        "Kotlin readln() should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "readln should be UserInput, got: {:?}",
        sources
    );

    // System.getenv -> EnvVar
    let sources = detect_sources("val key = System.getenv(\"API_KEY\")", 3, Language::Kotlin);
    assert!(
        !sources.is_empty(),
        "Kotlin System.getenv should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "System.getenv should be EnvVar, got: {:?}",
        sources
    );

    // request.getParameter -> HttpParam
    let sources = detect_sources(
        "val name = request.getParameter(\"name\")",
        4,
        Language::Kotlin,
    );
    assert!(
        !sources.is_empty(),
        "Kotlin request.getParameter should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "request.getParameter should be HttpParam, got: {:?}",
        sources
    );
}

#[test]
fn test_kotlin_detect_sinks() {
    // Runtime.getRuntime().exec -> ShellExec
    let sinks = detect_sinks("Runtime.getRuntime().exec(cmd)", 1, Language::Kotlin);
    assert!(
        !sinks.is_empty(),
        "Kotlin Runtime.exec should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "Runtime.exec should be ShellExec, got: {:?}",
        sinks
    );

    // ProcessBuilder -> ShellExec
    let sinks = detect_sinks("ProcessBuilder(cmd).start()", 2, Language::Kotlin);
    assert!(
        !sinks.is_empty(),
        "Kotlin ProcessBuilder should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "ProcessBuilder should be ShellExec, got: {:?}",
        sinks
    );

    // connection.prepareStatement -> SqlQuery
    let sinks = detect_sinks(
        "val stmt = connection.prepareStatement(sql)",
        3,
        Language::Kotlin,
    );
    assert!(
        !sinks.is_empty(),
        "Kotlin prepareStatement should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::SqlQuery),
        "prepareStatement should be SqlQuery, got: {:?}",
        sinks
    );
}

#[test]
fn test_kotlin_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("val n = val.toInt()", Language::Kotlin),
        Some(SanitizerType::Numeric),
        ".toInt() should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("val n = val.toLong()", Language::Kotlin),
        Some(SanitizerType::Numeric),
        ".toLong() should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("val n = val.toDouble()", Language::Kotlin),
        Some(SanitizerType::Numeric),
        ".toDouble() should be Numeric sanitizer"
    );
}

// =========================================================================
// Swift Taint Patterns
// =========================================================================

#[test]
fn test_swift_detect_sources() {
    // readLine() -> UserInput
    let sources = detect_sources("let input = readLine()", 1, Language::Swift);
    assert!(
        !sources.is_empty(),
        "Swift readLine() should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "readLine should be UserInput, got: {:?}",
        sources
    );

    // ProcessInfo.processInfo.environment -> EnvVar
    let sources = detect_sources(
        "let val = ProcessInfo.processInfo.environment[\"API_KEY\"]",
        2,
        Language::Swift,
    );
    assert!(
        !sources.is_empty(),
        "Swift ProcessInfo.environment should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "ProcessInfo.environment should be EnvVar, got: {:?}",
        sources
    );

    // URLSession -> UserInput (network)
    let sources = detect_sources(
        "let (data, _) = try await URLSession.shared.data(from: url)",
        3,
        Language::Swift,
    );
    assert!(
        !sources.is_empty(),
        "Swift URLSession should be detected as source"
    );

    // FileManager -> FileRead
    let sources = detect_sources(
        "let data = FileManager.default.contents(atPath: path)",
        4,
        Language::Swift,
    );
    assert!(
        !sources.is_empty(),
        "Swift FileManager should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::FileRead),
        "FileManager should be FileRead, got: {:?}",
        sources
    );
}

#[test]
fn test_swift_detect_sinks() {
    // Process() -> ShellExec
    let sinks = detect_sinks(
        "let proc = Process(); proc.executableURL = URL(fileURLWithPath: cmd)",
        1,
        Language::Swift,
    );
    assert!(
        !sinks.is_empty(),
        "Swift Process() should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "Process should be ShellExec, got: {:?}",
        sinks
    );

    // sqlite3_exec -> SqlQuery
    let sinks = detect_sinks("sqlite3_exec(db, sql, nil, nil, nil)", 2, Language::Swift);
    assert!(
        !sinks.is_empty(),
        "Swift sqlite3_exec should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::SqlQuery),
        "sqlite3_exec should be SqlQuery, got: {:?}",
        sinks
    );
}

#[test]
fn test_swift_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("let n = Int(val)", Language::Swift),
        Some(SanitizerType::Numeric),
        "Int() should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("let n = Double(val)", Language::Swift),
        Some(SanitizerType::Numeric),
        "Double() should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer(
            "let safe = val.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed)",
            Language::Swift
        ),
        Some(SanitizerType::Html),
        "addingPercentEncoding should be Html sanitizer"
    );
}

// =========================================================================
// C# Taint Patterns
// =========================================================================

#[test]
fn test_csharp_detect_sources() {
    // Console.ReadLine() -> UserInput
    let sources = detect_sources("string input = Console.ReadLine()", 1, Language::CSharp);
    assert!(
        !sources.is_empty(),
        "C# Console.ReadLine should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "Console.ReadLine should be UserInput, got: {:?}",
        sources
    );

    // Request.QueryString -> HttpParam
    let sources = detect_sources(
        "string id = Request.QueryString[\"id\"]",
        2,
        Language::CSharp,
    );
    assert!(
        !sources.is_empty(),
        "C# Request.QueryString should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "Request.QueryString should be HttpParam, got: {:?}",
        sources
    );

    // Request.Form -> HttpParam
    let sources = detect_sources("string name = Request.Form[\"name\"]", 3, Language::CSharp);
    assert!(
        !sources.is_empty(),
        "C# Request.Form should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "Request.Form should be HttpParam, got: {:?}",
        sources
    );

    // Environment.GetEnvironmentVariable -> EnvVar
    let sources = detect_sources(
        "string key = Environment.GetEnvironmentVariable(\"KEY\")",
        4,
        Language::CSharp,
    );
    assert!(
        !sources.is_empty(),
        "C# Environment.GetEnvironmentVariable should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "GetEnvironmentVariable should be EnvVar, got: {:?}",
        sources
    );
}

#[test]
fn test_csharp_detect_sinks() {
    // Process.Start -> ShellExec
    let sinks = detect_sinks("Process.Start(cmd)", 1, Language::CSharp);
    assert!(
        !sinks.is_empty(),
        "C# Process.Start should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "Process.Start should be ShellExec, got: {:?}",
        sinks
    );

    // SqlCommand -> SqlQuery
    let sinks = detect_sinks("var cmd = new SqlCommand(sql, conn)", 2, Language::CSharp);
    assert!(
        !sinks.is_empty(),
        "C# SqlCommand should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::SqlQuery),
        "SqlCommand should be SqlQuery, got: {:?}",
        sinks
    );

    // Activator.CreateInstance -> CodeEval (reflection)
    let sinks = detect_sinks("Activator.CreateInstance(typeName)", 3, Language::CSharp);
    assert!(
        !sinks.is_empty(),
        "C# Activator.CreateInstance should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "Activator.CreateInstance should be CodeEval, got: {:?}",
        sinks
    );
}

#[test]
fn test_csharp_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("int n = int.Parse(val)", Language::CSharp),
        Some(SanitizerType::Numeric),
        "int.Parse should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("int n = Convert.ToInt32(val)", Language::CSharp),
        Some(SanitizerType::Numeric),
        "Convert.ToInt32 should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer(
            "string safe = HttpUtility.HtmlEncode(val)",
            Language::CSharp
        ),
        Some(SanitizerType::Html),
        "HttpUtility.HtmlEncode should be Html sanitizer"
    );
}

// =========================================================================
// Scala Taint Patterns
// =========================================================================

#[test]
fn test_scala_detect_sources() {
    // scala.io.StdIn.readLine() -> UserInput
    let sources = detect_sources("val input = scala.io.StdIn.readLine()", 1, Language::Scala);
    assert!(
        !sources.is_empty(),
        "Scala StdIn.readLine should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "StdIn.readLine should be UserInput, got: {:?}",
        sources
    );

    // System.getenv -> EnvVar
    let sources = detect_sources("val key = System.getenv(\"API_KEY\")", 2, Language::Scala);
    assert!(
        !sources.is_empty(),
        "Scala System.getenv should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "System.getenv should be EnvVar, got: {:?}",
        sources
    );

    // Source.fromFile -> FileRead
    let sources = detect_sources(
        "val content = Source.fromFile(path).mkString",
        3,
        Language::Scala,
    );
    assert!(
        !sources.is_empty(),
        "Scala Source.fromFile should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::FileRead),
        "Source.fromFile should be FileRead, got: {:?}",
        sources
    );
}

#[test]
fn test_scala_detect_sinks() {
    // Runtime.getRuntime.exec -> ShellExec
    let sinks = detect_sinks("Runtime.getRuntime.exec(cmd)", 1, Language::Scala);
    assert!(
        !sinks.is_empty(),
        "Scala Runtime.exec should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "Runtime.exec should be ShellExec, got: {:?}",
        sinks
    );

    // sys.process -> ShellExec
    let sinks = detect_sinks("import sys.process._; cmd.!", 2, Language::Scala);
    assert!(
        !sinks.is_empty(),
        "Scala sys.process should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "sys.process should be ShellExec, got: {:?}",
        sinks
    );

    // stmt.execute -> SqlQuery
    let sinks = detect_sinks("stmt.execute(sql)", 3, Language::Scala);
    assert!(
        !sinks.is_empty(),
        "Scala stmt.execute should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::SqlQuery),
        "stmt.execute should be SqlQuery, got: {:?}",
        sinks
    );
}

#[test]
fn test_scala_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("val n = val.toInt", Language::Scala),
        Some(SanitizerType::Numeric),
        ".toInt should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("val n = val.toLong", Language::Scala),
        Some(SanitizerType::Numeric),
        ".toLong should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer(
            "val safe = StringEscapeUtils.escapeHtml4(val)",
            Language::Scala
        ),
        Some(SanitizerType::Html),
        "StringEscapeUtils.escapeHtml4 should be Html sanitizer"
    );
}

// =========================================================================
// PHP Taint Patterns
// =========================================================================

#[test]
fn test_php_detect_sources() {
    // $_GET -> HttpParam
    let sources = detect_sources("$name = $_GET['name']", 1, Language::Php);
    assert!(
        !sources.is_empty(),
        "PHP $_GET should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "$_GET should be HttpParam, got: {:?}",
        sources
    );

    // $_POST -> HttpBody
    let sources = detect_sources("$data = $_POST['data']", 2, Language::Php);
    assert!(
        !sources.is_empty(),
        "PHP $_POST should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpBody),
        "$_POST should be HttpBody, got: {:?}",
        sources
    );

    // $_REQUEST -> HttpParam
    let sources = detect_sources("$val = $_REQUEST['key']", 3, Language::Php);
    assert!(
        !sources.is_empty(),
        "PHP $_REQUEST should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "$_REQUEST should be HttpParam, got: {:?}",
        sources
    );

    // $_COOKIE -> HttpParam
    let sources = detect_sources("$session = $_COOKIE['session']", 4, Language::Php);
    assert!(
        !sources.is_empty(),
        "PHP $_COOKIE should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::HttpParam),
        "$_COOKIE should be HttpParam, got: {:?}",
        sources
    );

    // fgets -> UserInput
    let sources = detect_sources("$line = fgets(STDIN)", 5, Language::Php);
    assert!(
        !sources.is_empty(),
        "PHP fgets should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "fgets should be UserInput, got: {:?}",
        sources
    );

    // file_get_contents -> FileRead
    let sources = detect_sources("$content = file_get_contents($path)", 6, Language::Php);
    assert!(
        !sources.is_empty(),
        "PHP file_get_contents should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::FileRead),
        "file_get_contents should be FileRead, got: {:?}",
        sources
    );
}

#[test]
fn test_php_detect_sinks() {
    // eval -> CodeEval
    let sinks = detect_sinks("eval($code)", 1, Language::Php);
    assert!(!sinks.is_empty(), "PHP eval should be detected as sink");
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "eval should be CodeEval, got: {:?}",
        sinks
    );

    // exec -> ShellExec
    let sinks = detect_sinks("exec($cmd, $output)", 2, Language::Php);
    assert!(!sinks.is_empty(), "PHP exec should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "exec should be ShellExec, got: {:?}",
        sinks
    );

    // system -> ShellExec
    let sinks = detect_sinks("system($cmd)", 3, Language::Php);
    assert!(!sinks.is_empty(), "PHP system should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "system should be ShellExec, got: {:?}",
        sinks
    );

    // shell_exec -> ShellExec
    let sinks = detect_sinks("$output = shell_exec($cmd)", 4, Language::Php);
    assert!(
        !sinks.is_empty(),
        "PHP shell_exec should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "shell_exec should be ShellExec, got: {:?}",
        sinks
    );

    // passthru -> ShellExec
    let sinks = detect_sinks("passthru($cmd)", 5, Language::Php);
    assert!(!sinks.is_empty(), "PHP passthru should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "passthru should be ShellExec, got: {:?}",
        sinks
    );

    // mysqli_query -> SqlQuery
    let sinks = detect_sinks("$result = mysqli_query($conn, $sql)", 6, Language::Php);
    assert!(
        !sinks.is_empty(),
        "PHP mysqli_query should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::SqlQuery),
        "mysqli_query should be SqlQuery, got: {:?}",
        sinks
    );

    // popen -> ShellExec
    let sinks = detect_sinks("$handle = popen($cmd, 'r')", 7, Language::Php);
    assert!(!sinks.is_empty(), "PHP popen should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "popen should be ShellExec, got: {:?}",
        sinks
    );
}

#[test]
fn test_php_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("$n = intval($val)", Language::Php),
        Some(SanitizerType::Numeric),
        "intval should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("$n = (int)$val", Language::Php),
        Some(SanitizerType::Numeric),
        "(int) cast should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("$safe = htmlspecialchars($val)", Language::Php),
        Some(SanitizerType::Html),
        "htmlspecialchars should be Html sanitizer"
    );
    assert_eq!(
        detect_sanitizer(
            "$safe = mysqli_real_escape_string($conn, $val)",
            Language::Php
        ),
        Some(SanitizerType::Shell),
        "mysqli_real_escape_string should be Shell sanitizer"
    );
}

// =========================================================================
// Lua Taint Patterns
// =========================================================================

#[test]
fn test_lua_detect_sources() {
    // io.read() -> UserInput
    let sources = detect_sources("local input = io.read()", 1, Language::Lua);
    assert!(
        !sources.is_empty(),
        "Lua io.read() should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "io.read should be UserInput, got: {:?}",
        sources
    );

    // os.getenv -> EnvVar
    let sources = detect_sources("local val = os.getenv(\"HOME\")", 2, Language::Lua);
    assert!(
        !sources.is_empty(),
        "Lua os.getenv should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "os.getenv should be EnvVar, got: {:?}",
        sources
    );

    // io.open + :read -> FileRead
    let sources = detect_sources("local f = io.open(path, \"r\")", 3, Language::Lua);
    assert!(
        !sources.is_empty(),
        "Lua io.open should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::FileRead),
        "io.open should be FileRead, got: {:?}",
        sources
    );
}

#[test]
fn test_lua_detect_sinks() {
    // os.execute -> ShellExec
    let sinks = detect_sinks("os.execute(cmd)", 1, Language::Lua);
    assert!(
        !sinks.is_empty(),
        "Lua os.execute should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "os.execute should be ShellExec, got: {:?}",
        sinks
    );

    // io.popen -> ShellExec
    let sinks = detect_sinks("local handle = io.popen(cmd)", 2, Language::Lua);
    assert!(!sinks.is_empty(), "Lua io.popen should be detected as sink");
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "io.popen should be ShellExec, got: {:?}",
        sinks
    );

    // loadstring -> CodeEval
    let sinks = detect_sinks("local fn = loadstring(code)", 3, Language::Lua);
    assert!(
        !sinks.is_empty(),
        "Lua loadstring should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "loadstring should be CodeEval, got: {:?}",
        sinks
    );

    // load -> CodeEval
    let sinks = detect_sinks("local fn = load(code)", 4, Language::Lua);
    assert!(!sinks.is_empty(), "Lua load should be detected as sink");
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "load should be CodeEval, got: {:?}",
        sinks
    );
}

#[test]
fn test_lua_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("local n = tonumber(val)", Language::Lua),
        Some(SanitizerType::Numeric),
        "tonumber should be Numeric sanitizer"
    );
}

// =========================================================================
// Luau Taint Patterns (shares patterns with Lua)
// =========================================================================

#[test]
fn test_luau_detect_sources() {
    // io.read() -> UserInput
    let sources = detect_sources("local input = io.read()", 1, Language::Luau);
    assert!(
        !sources.is_empty(),
        "Luau io.read() should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "io.read should be UserInput, got: {:?}",
        sources
    );

    // os.getenv -> EnvVar
    let sources = detect_sources("local val = os.getenv(\"HOME\")", 2, Language::Luau);
    assert!(
        !sources.is_empty(),
        "Luau os.getenv should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "os.getenv should be EnvVar, got: {:?}",
        sources
    );
}

#[test]
fn test_luau_detect_sinks() {
    // os.execute -> ShellExec
    let sinks = detect_sinks("os.execute(cmd)", 1, Language::Luau);
    assert!(
        !sinks.is_empty(),
        "Luau os.execute should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "os.execute should be ShellExec, got: {:?}",
        sinks
    );

    // loadstring -> CodeEval
    let sinks = detect_sinks("local fn = loadstring(code)", 2, Language::Luau);
    assert!(
        !sinks.is_empty(),
        "Luau loadstring should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "loadstring should be CodeEval, got: {:?}",
        sinks
    );
}

#[test]
fn test_luau_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("local n = tonumber(val)", Language::Luau),
        Some(SanitizerType::Numeric),
        "tonumber should be Numeric sanitizer"
    );
}

// =========================================================================
// Elixir Taint Patterns
// =========================================================================

#[test]
fn test_elixir_detect_sources() {
    // IO.gets -> UserInput
    let sources = detect_sources("input = IO.gets(\"Enter: \")", 1, Language::Elixir);
    assert!(
        !sources.is_empty(),
        "Elixir IO.gets should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "IO.gets should be UserInput, got: {:?}",
        sources
    );

    // System.get_env -> EnvVar
    let sources = detect_sources("key = System.get_env(\"API_KEY\")", 2, Language::Elixir);
    assert!(
        !sources.is_empty(),
        "Elixir System.get_env should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "System.get_env should be EnvVar, got: {:?}",
        sources
    );

    // File.read -> FileRead
    let sources = detect_sources("{:ok, content} = File.read(path)", 3, Language::Elixir);
    assert!(
        !sources.is_empty(),
        "Elixir File.read should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::FileRead),
        "File.read should be FileRead, got: {:?}",
        sources
    );
}

#[test]
fn test_elixir_detect_sinks() {
    // System.cmd -> ShellExec
    let sinks = detect_sinks("System.cmd(cmd, args)", 1, Language::Elixir);
    assert!(
        !sinks.is_empty(),
        "Elixir System.cmd should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "System.cmd should be ShellExec, got: {:?}",
        sinks
    );

    // Code.eval_string -> CodeEval
    let sinks = detect_sinks("Code.eval_string(code)", 2, Language::Elixir);
    assert!(
        !sinks.is_empty(),
        "Elixir Code.eval_string should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "Code.eval_string should be CodeEval, got: {:?}",
        sinks
    );

    // Ecto.Adapters.SQL.query -> SqlQuery
    let sinks = detect_sinks("Ecto.Adapters.SQL.query(Repo, sql)", 3, Language::Elixir);
    assert!(
        !sinks.is_empty(),
        "Elixir Ecto.Adapters.SQL.query should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::SqlQuery),
        "Ecto.Adapters.SQL.query should be SqlQuery, got: {:?}",
        sinks
    );
}

#[test]
fn test_elixir_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("n = String.to_integer(val)", Language::Elixir),
        Some(SanitizerType::Numeric),
        "String.to_integer should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("safe = Phoenix.HTML.html_escape(val)", Language::Elixir),
        Some(SanitizerType::Html),
        "Phoenix.HTML.html_escape should be Html sanitizer"
    );
}

// =========================================================================
// OCaml Taint Patterns
// =========================================================================

#[test]
fn test_ocaml_detect_sources() {
    // read_line() -> UserInput
    let sources = detect_sources("let input = read_line ()", 1, Language::Ocaml);
    assert!(
        !sources.is_empty(),
        "OCaml read_line should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "read_line should be UserInput, got: {:?}",
        sources
    );

    // Sys.getenv -> EnvVar
    let sources = detect_sources("let key = Sys.getenv \"API_KEY\"", 2, Language::Ocaml);
    assert!(
        !sources.is_empty(),
        "OCaml Sys.getenv should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::EnvVar),
        "Sys.getenv should be EnvVar, got: {:?}",
        sources
    );

    // input_line -> UserInput
    let sources = detect_sources("let line = input_line ic", 3, Language::Ocaml);
    assert!(
        !sources.is_empty(),
        "OCaml input_line should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "input_line should be UserInput, got: {:?}",
        sources
    );

    // In_channel.read_all -> FileRead
    let sources = detect_sources("let content = In_channel.read_all path", 4, Language::Ocaml);
    assert!(
        !sources.is_empty(),
        "OCaml In_channel.read_all should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::FileRead),
        "In_channel.read_all should be FileRead, got: {:?}",
        sources
    );
}

#[test]
fn test_ocaml_detect_sinks() {
    // Sys.command -> ShellExec
    let sinks = detect_sinks("let _ = Sys.command cmd", 1, Language::Ocaml);
    assert!(
        !sinks.is_empty(),
        "OCaml Sys.command should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "Sys.command should be ShellExec, got: {:?}",
        sinks
    );

    // Unix.execvp -> ShellExec
    let sinks = detect_sinks("Unix.execvp cmd args", 2, Language::Ocaml);
    assert!(
        !sinks.is_empty(),
        "OCaml Unix.execvp should be detected as sink"
    );
    assert!(
        sinks
            .iter()
            .any(|s| s.sink_type == TaintSinkType::ShellExec),
        "Unix.execvp should be ShellExec, got: {:?}",
        sinks
    );

    // Sqlite3.exec -> SqlQuery
    let sinks = detect_sinks("Sqlite3.exec db sql", 3, Language::Ocaml);
    assert!(
        !sinks.is_empty(),
        "OCaml Sqlite3.exec should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::SqlQuery),
        "Sqlite3.exec should be SqlQuery, got: {:?}",
        sinks
    );
}

#[test]
fn test_ocaml_detect_sanitizers() {
    assert_eq!(
        detect_sanitizer("let n = int_of_string val", Language::Ocaml),
        Some(SanitizerType::Numeric),
        "int_of_string should be Numeric sanitizer"
    );
    assert_eq!(
        detect_sanitizer("let f = float_of_string val", Language::Ocaml),
        Some(SanitizerType::Numeric),
        "float_of_string should be Numeric sanitizer"
    );
}

// =============================================================================
// Section 11: AST-Based Detection Tests (Phase 9)
//
// These tests verify that AST-based taint detection correctly:
// 1. Detects sources/sinks/sanitizers in actual code
// 2. Rejects false positives from comments and string literals
// 3. Works with compute_taint_with_tree
// =============================================================================

use super::taint::{
    compute_taint_with_tree, detect_sinks_ast, detect_sources_ast,
};
use crate::ast::parser::ParserPool;

// =========================================================================
// AST False Positive Rejection Tests
// =========================================================================

/// Tests that eval() in a Python comment is NOT detected as a sink
#[test]
fn test_ast_python_eval_in_comment_not_sink() {
    let pool = ParserPool::new();
    let source = "# eval(user_code) - dangerous, don't use\nx = 1";
    let tree = pool.parse(source, Language::Python).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::Python, None);
    assert!(
        sinks.is_empty(),
        "eval in comment should NOT be detected as sink, got: {:?}",
        sinks
    );
}

/// Tests that input() in a Python string is NOT detected as a source
#[test]
fn test_ast_python_input_in_string_not_source() {
    let pool = ParserPool::new();
    let source = "msg = \"use input() to get data\"";
    let tree = pool.parse(source, Language::Python).unwrap();
    let root = tree.root_node();

    let sources = detect_sources_ast(&root, source.as_bytes(), Language::Python, None);
    // Should not detect input() inside a string as a source
    assert!(
        sources.is_empty(),
        "input() in string should NOT be detected as source, got: {:?}",
        sources
    );
}

/// Tests that eval() in actual Python code IS detected as a sink
#[test]
fn test_ast_python_eval_in_code_is_sink() {
    let pool = ParserPool::new();
    let source = "result = eval(user_code)";
    let tree = pool.parse(source, Language::Python).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::Python, None);
    assert!(
        !sinks.is_empty(),
        "eval in actual code should be detected as sink"
    );
    assert!(
        sinks.iter().any(|s| s.sink_type == TaintSinkType::CodeEval),
        "eval should be CodeEval, got: {:?}",
        sinks
    );
}

/// Tests that input() in actual Python code IS detected as a source
#[test]
fn test_ast_python_input_in_code_is_source() {
    let pool = ParserPool::new();
    let source = "user_input = input()";
    let tree = pool.parse(source, Language::Python).unwrap();
    let root = tree.root_node();

    let sources = detect_sources_ast(&root, source.as_bytes(), Language::Python, None);
    assert!(
        !sources.is_empty(),
        "input() in code should be detected as source"
    );
    assert!(
        sources
            .iter()
            .any(|s| s.source_type == TaintSourceType::UserInput),
        "input should be UserInput, got: {:?}",
        sources
    );
    assert_eq!(sources[0].var, "user_input");
}

/// Tests that os.system() in a Python comment is NOT detected as a sink
#[test]
fn test_ast_python_os_system_in_comment_not_sink() {
    let pool = ParserPool::new();
    let source = "# os.system(cmd) is dangerous\nresult = 42";
    let tree = pool.parse(source, Language::Python).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::Python, None);
    assert!(
        sinks.is_empty(),
        "os.system in comment should NOT be detected as sink"
    );
}

// =========================================================================
// AST-Based Detection for TypeScript
// =========================================================================

#[test]
fn test_ast_typescript_eval_in_comment_not_sink() {
    let pool = ParserPool::new();
    let source = "// eval(code) - never do this\nconst x = 1;";
    let tree = pool.parse(source, Language::TypeScript).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::TypeScript, None);
    assert!(
        sinks.is_empty(),
        "eval in TS comment should NOT be detected as sink"
    );
}

#[test]
fn test_ast_typescript_eval_in_code_is_sink() {
    let pool = ParserPool::new();
    let source = "eval(userInput);";
    let tree = pool.parse(source, Language::TypeScript).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::TypeScript, None);
    assert!(
        !sinks.is_empty(),
        "eval in TS code should be detected as sink"
    );
}

// =========================================================================
// AST-Based Detection for Go
// =========================================================================

#[test]
fn test_ast_go_exec_command_in_comment_not_sink() {
    let pool = ParserPool::new();
    let source = "package main\n// exec.Command(cmd) is dangerous\nfunc main() {}";
    let tree = pool.parse(source, Language::Go).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::Go, None);
    assert!(
        sinks.is_empty(),
        "exec.Command in Go comment should NOT be detected as sink"
    );
}

// =========================================================================
// AST-Based Detection for C
// =========================================================================

#[test]
fn test_ast_c_system_in_comment_not_sink() {
    let pool = ParserPool::new();
    let source = "// system(cmd) is dangerous\nint main() { return 0; }";
    let tree = pool.parse(source, Language::C).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::C, None);
    assert!(
        sinks.is_empty(),
        "system() in C comment should NOT be detected as sink"
    );
}

#[test]
fn test_ast_c_system_in_code_is_sink() {
    let pool = ParserPool::new();
    let source = "int main() { system(cmd); return 0; }";
    let tree = pool.parse(source, Language::C).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::C, None);
    assert!(
        !sinks.is_empty(),
        "system() in C code should be detected as sink"
    );
}

// =========================================================================
// AST-Based Detection for Java
// =========================================================================

#[test]
fn test_ast_java_runtime_exec_in_comment_not_sink() {
    let pool = ParserPool::new();
    let source = "class Main {\n// Runtime.getRuntime().exec(cmd)\nvoid f() {} }";
    let tree = pool.parse(source, Language::Java).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::Java, None);
    assert!(
        sinks.is_empty(),
        "Runtime.exec in Java comment should NOT be detected as sink"
    );
}

// =========================================================================
// AST-Based Detection for Rust
// =========================================================================

#[test]
fn test_ast_rust_command_in_comment_not_sink() {
    let pool = ParserPool::new();
    let source = "fn main() {\n// Command::new(cmd).spawn()\nlet x = 1;\n}";
    let tree = pool.parse(source, Language::Rust).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::Rust, None);
    assert!(
        sinks.is_empty(),
        "Command::new in Rust comment should NOT be detected as sink"
    );
}

// =========================================================================
// AST-Based Detection for Ruby
// =========================================================================

#[test]
fn test_ast_ruby_eval_in_comment_not_sink() {
    let pool = ParserPool::new();
    let source = "# eval(code) is dangerous\nx = 1";
    let tree = pool.parse(source, Language::Ruby).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::Ruby, None);
    assert!(
        sinks.is_empty(),
        "eval in Ruby comment should NOT be detected as sink"
    );
}

// =========================================================================
// AST-Based Detection for PHP
// =========================================================================

#[test]
fn test_ast_php_eval_in_comment_not_sink() {
    let pool = ParserPool::new();
    let source = "<?php\n// eval($code) is dangerous\n$x = 1;\n?>";
    let tree = pool.parse(source, Language::Php).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::Php, None);
    assert!(
        sinks.is_empty(),
        "eval in PHP comment should NOT be detected as sink"
    );
}

// =========================================================================
// AST-Based Detection for Lua
// =========================================================================

#[test]
fn test_ast_lua_loadstring_in_comment_not_sink() {
    let pool = ParserPool::new();
    let source = "-- loadstring(code) is dangerous\nlocal x = 1";
    let tree = pool.parse(source, Language::Lua).unwrap();
    let root = tree.root_node();

    let sinks = detect_sinks_ast(&root, source.as_bytes(), Language::Lua, None);
    assert!(
        sinks.is_empty(),
        "loadstring in Lua comment should NOT be detected as sink"
    );
}

// =========================================================================
// compute_taint_with_tree Tests
// =========================================================================

/// Tests that compute_taint_with_tree works without a tree (regex fallback)
#[test]
fn test_compute_taint_with_tree_no_tree() {
    use fixtures::*;

    let cfg = linear_cfg();
    let refs = vec![make_def("x", 1), make_use("x", 5)];

    let mut statements = HashMap::new();
    statements.insert(1, "x = input()".to_string());
    statements.insert(5, "eval(x)".to_string());

    // Without tree - should behave like compute_taint
    let result =
        compute_taint_with_tree(&cfg, &refs, &statements, None, None, Language::Python).unwrap();

    assert!(
        !result.sources.is_empty(),
        "Should detect source via regex fallback"
    );
    assert!(
        !result.sinks.is_empty(),
        "Should detect sink via regex fallback"
    );
}

/// Tests that compute_taint_with_tree detects SQL injection with a tree
#[test]
fn test_compute_taint_with_tree_sql_injection() {
    use fixtures::*;

    let pool = ParserPool::new();
    let source_code = "user_input = input()\nquery = \"SELECT * FROM users WHERE id = \" + user_input\ncursor.execute(query)";
    let tree = pool.parse(source_code, Language::Python).unwrap();

    let cfg = linear_cfg();
    let refs = vec![
        make_def("user_input", 1),
        make_use("user_input", 2),
        make_def("query", 2),
        make_use("query", 3),
    ];

    let mut statements = HashMap::new();
    statements.insert(1, "user_input = input()".to_string());
    statements.insert(
        2,
        "query = \"SELECT * FROM users WHERE id = \" + user_input".to_string(),
    );
    statements.insert(3, "cursor.execute(query)".to_string());

    let result = compute_taint_with_tree(
        &cfg,
        &refs,
        &statements,
        Some(&tree),
        Some(source_code.as_bytes()),
        Language::Python,
    )
    .unwrap();

    assert!(
        !result.sources.is_empty(),
        "Should detect input() as source"
    );
}

/// Tests that AST detection for all 18 languages has pattern coverage
#[test]
fn test_ast_patterns_defined_for_all_languages() {
    let languages = vec![
        Language::Python,
        Language::TypeScript,
        Language::JavaScript,
        Language::Go,
        Language::Rust,
        Language::Java,
        Language::C,
        Language::Cpp,
        Language::Ruby,
        Language::Kotlin,
        Language::Swift,
        Language::CSharp,
        Language::Scala,
        Language::Php,
        Language::Lua,
        Language::Luau,
        Language::Elixir,
        Language::Ocaml,
    ];

    for lang in languages {
        let patterns = super::taint::get_patterns(lang);
        assert!(
            !patterns.sources.is_empty() || !patterns.sinks.is_empty(),
            "Language {:?} should have at least some patterns defined",
            lang
        );
    }
}
