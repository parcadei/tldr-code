//! The cross-language contract for anonymous-callback definitions: a `kind: "call"` definition
//! appears for EVERY language that has an anonymous-callable form, and for none that doesn't.
//!
//! Asserted through the PUBLIC path (`get_code_structure` on a real file) rather than against the
//! extractor's internals, because that is what the CLI, the daemon, and fastedit all consume — a
//! test against the private helper could pass while the shipped surface emitted nothing.
//!
//! Each sample is written in the idiom that language's own test framework or stdlib actually
//! uses, so a grammar change that breaks the REAL shape fails here rather than passing against a
//! synthetic one. C is present deliberately, expecting none: without a negative case the suite
//! would still pass if every language started emitting a callback for every call.
//!
//! Run: cargo test -p tldr-core --test anon_callback_coverage -- --nocapture

use std::fs;
use tldr_core::{get_code_structure, IgnoreSpec, Language};

fn samples() -> Vec<(Language, &'static str, &'static str, &'static str)> {
    vec![
        // (language, file extension, source, the name we expect to see)
        (Language::JavaScript, "js", "suiteSetup(async function () {\n  const a = 1;\n  return a;\n});\n", "suiteSetup"),
        (Language::TypeScript, "ts", "test('a title here', async () => {\n  const a: number = 1;\n  return a;\n});\n", "test:a-title-here"),
        (Language::Python, "py", "sorted(items, key=lambda a:\n    a.b\n)\n", "sorted"),
        (Language::Ruby, "rb", "it 'does a thing' do\n  expect(1).to eq(1)\nend\n", "it:does-a-thing"),
        (Language::Go, "go", "func main() {\n\thttp.HandleFunc(\"/x\", func(w int, r int) {\n\t\tprintln(w)\n\t})\n}\n", "HandleFunc:x"),
        (Language::Rust, "rs", "fn main() {\n    thread::spawn(|| {\n        println!(\"x\");\n    });\n}\n", "spawn"),
        (Language::Java, "java", "class A {\n  void m() {\n    list.forEach(x -> {\n      System.out.println(x);\n    });\n  }\n}\n", "forEach"),
        (Language::CSharp, "cs", "class A {\n  void M() {\n    list.ForEach(x => {\n      Console.WriteLine(x);\n    });\n  }\n}\n", "ForEach"),
        (Language::Cpp, "cpp", "int main() {\n  run([](int x) {\n    return x + 1;\n  });\n}\n", "run"),
        (Language::Kotlin, "kt", "fun main() {\n  runBlocking {\n    delay(1)\n  }\n}\n", "runBlocking"),
        (Language::Swift, "swift", "describe(\"a thing\") {\n  let x = 1\n  print(x)\n}\n", "describe:a-thing"),
        (Language::Scala, "scala", "object A {\n  list.foreach { x =>\n    println(x)\n  }\n}\n", "foreach"),
        (Language::Php, "php", "<?php\n$f = array_map(function ($x) {\n    return $x + 1;\n}, $items);\n", "array_map"),
        (Language::Lua, "lua", "setup(function ()\n  local a = 1\n  return a\nend)\n", "setup"),
        (Language::Luau, "luau", "setup(function ()\n  local a = 1\n  return a\nend)\n", "setup"),
        (Language::Elixir, "ex", "test \"a title\" do\n  assert 1 == 1\nend\n", "test:a-title"),
        (Language::Ocaml, "ml", "let () =\n  List.iter (fun x ->\n    print_int x\n  ) items\n", "iter"),
        (Language::C, "c", "int main(void) {\n  return 0;\n}\n", ""), // no lambda form: expect none
    ]
}

#[test]
fn call_definitions_cover_every_language_that_has_anonymous_callables() {
    // PID-scoped: cargo runs tests as parallel threads in ONE process, and a shared fixed path
    // would let a concurrent run of this binary overwrite the sample another case is reading.
    let dir = std::env::temp_dir().join(format!("tldr-anon-coverage-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let mut missing: Vec<String> = Vec::new();

    println!("\n{:<12} {:<28} EMITTED kind=call", "LANG", "EXPECTED");
    println!("{}", "-".repeat(100));
    for (lang, ext, src, expected) in samples() {
        let path = dir.join(format!("probe.{ext}"));
        fs::write(&path, src).expect("write sample");
        let st =
            get_code_structure(&path, lang, 0, Some(&IgnoreSpec::default())).expect("structure");
        let calls: Vec<String> = st
            .files
            .iter()
            .flat_map(|f| f.definitions.iter())
            .filter(|d| d.kind == "call")
            .map(|d| format!("{} (L{}-{})", d.name, d.line_start, d.line_end))
            .collect();
        println!(
            "{:<12} {:<28} {}",
            format!("{lang:?}"),
            expected,
            calls.join(" | ")
        );
        if !expected.is_empty() && !calls.iter().any(|c| c.starts_with(expected)) {
            missing.push(format!("{lang:?} (wanted {expected})"));
        }
        if expected.is_empty() && !calls.is_empty() {
            missing.push(format!("{lang:?} emitted unexpected {calls:?}"));
        }
    }
    println!();
    assert!(missing.is_empty(), "languages not covered: {missing:?}");
}
