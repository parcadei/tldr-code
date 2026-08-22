---
name: tldr-code
description: >
  Token-efficient code analysis (READ, via `tldr`) AND AST-scoped editing (WRITE, via `fastedit` — edit/insert/rename/move/delete/refactor a symbol without repeating old code) for 18 languages
  (Python, TypeScript, JavaScript, Go, Rust, Java, C, C++, Ruby, Kotlin, Swift,
  C#, Scala, PHP, Lua, Luau, Elixir, OCaml). Reach for it BEFORE reading whole
  files or editing unfamiliar code: it extracts ONLY the lines that define a
  symbol, that it calls, or that call it — plus call graphs, reverse-impact,
  program slices, taint/security flows, complexity metrics, dead code, design
  patterns, and BM25 + natural-language semantic search. Invoke it INTENTIONALLY
  (you choose what to query) — it is dramatically cheaper than dumping source
  into context. Use when you need to understand, navigate, locate, or assess
  impact in a codebase: "where is X defined / who calls X / what breaks if I
  change X / show me only the code that affects line N / is this input tainted /
  what's the structure of this module / find dead code / find the function that
  does Y".
---

# tldr — surgical, token-efficient code analysis

`tldr` (repo: parcadei/tldr-code, Rust, AGPL-3.0) parses code with tree-sitter
into a knowledge graph and answers **structural** questions as compact JSON/text.
It is the single best tool for **exploring a codebase and extracting the exact
slice of code that matters** — instead of reading entire files into context.

**The core idea — intentional & surgical.** Do NOT dump a whole file to "see how
it works." Ask `tldr` the precise question and it returns ONLY the relevant
lines: the symbol's definition, the lines it depends on, the lines that depend on
it, the callers, the slice. You decide the query; `tldr` returns the signal.

> `tldr` is invoked **deliberately by you**. It is NOT a passive interceptor.
> The generic output-compression layer is **distill** (`cmd | distill "<prompt>"`),
> which wraps command OUTPUT indiscriminately. `tldr` is the opposite: a precise
> instrument you reach for on purpose. The two coexist — `distill` makes a call's
> output cheaper; `tldr` makes you ask a better question in the first place.

## When to reach for tldr (instead of Read/Grep)

| You want to… | Don't | Do |
|---|---|---|
| Understand a module | Read the whole file | `tldr structure <path>` |
| Find where X is defined | Grep + read | `tldr definition --symbol X --file <f>` (`--symbol` needs `--file`) |
| See every caller of X | Grep the name | `tldr references X <path>` / `tldr impact X <path>` |
| Know what breaks if you change X | Guess | `tldr whatbreaks X <path>` |
| Get only the code that affects line N | Read the file | `tldr slice <file> <fn> <N>` |
| Understand one function fully | Read around it | `tldr structure <file>` + `tldr impact <fn> <path>` (fast) — or `tldr explain <file> <fn>` (⚠ slow on large repos, see Performance) |
| Trace how data reaches a sink | Read everything | `tldr taint <file> <fn>` / `tldr vuln <path>` |
| Find the function that does Y | Grep keywords | `tldr semantic 'Y' <path>` |
| Assess a codebase's health | Skim files | `tldr health <path>` |
| Find dead code before deleting | Manual audit | `tldr dead <path>` |

**Default to `tldr` first when navigating or assessing code you don't already
have open.** Read the actual file only once `tldr` has pinned the exact lines.

## Killer intentional recipes

```bash
# BEFORE EDITING a symbol — get its definition, its callers, and blast radius:
tldr definition --symbol parse_config --file src/config.py --project .   # by NAME: --symbol REQUIRES --file
                                          # (positional form is `tldr definition <FILE> <LINE> <COLUMN>`)
tldr impact parse_config src/             # who calls it (reverse call graph)
tldr whatbreaks parse_config src/         # what breaks if its behavior changes
tldr explain src/config.py parse_config   # signature + purity + complexity + callers/callees (FILE then FUNCTION)
                                          # ⚠ builds the FULL call graph — can take MINUTES on a large repo.
                                          # For most needs, structure (signature) + impact (callers) is seconds.

# EXTRACT ONLY the lines that affect a specific line (backward program slice):
tldr slice src/auth.py authenticate 142        # only the statements that influence L142
tldr chop src/auth.py authenticate 142 150     # forward(from L142) ∩ backward(to L150) — needs TWO lines

# UNDERSTAND a subsystem without reading it:
tldr structure src/payments/              # functions, classes, imports per file
tldr context handle_request --project .   # LLM-ready context graph from an entry point
tldr calls src/                           # cross-file call graph

# FIND code by meaning (semantic feature — installed):
tldr semantic 'where do we validate the JWT signature' src/
tldr search 'retry.*backoff' src/         # BM25 + structure + call-graph context cards

# SECURITY / CORRECTNESS sweeps:
tldr taint src/auth.py authenticate       # injection/XSS taint flows (FILE + FUNCTION, not a dir)
tldr vuln src/                            # SQLi, XSS, command injection (path-wide)
tldr secure src/                          # security dashboard (taint+resources+bounds+contracts)
tldr resources src/                       # leaks, double-close, use-after-close
tldr api-check src/                       # missing timeouts, bare except, weak crypto, unclosed files

# QUALITY / REFACTOR triage:
tldr health src/                          # one-shot health dashboard
tldr smells src/ ; tldr hotspots src/ ; tldr cognitive src/     # path-wide
tldr dead src/ ; tldr clones src/ ; tldr todo src/              # cleanup targets
tldr complexity src/auth.py authenticate  # ⚠ per-FUNCTION, not per-path (unlike its siblings above)
```

## Argument shapes that surprise people

Most commands take `<PATH>`. These do NOT — getting them wrong yields a confusing
"required arguments were not provided", or silently misreads your symbol as a filename:

| Command | Real shape | Trap |
|---|---|---|
| `definition` | `--symbol X --file <f>` (or positional `<FILE> <LINE> <COLUMN>`) | `--symbol` **requires** `--file`; `tldr definition X src/` reads `X` as the FILE |
| `explain` | `<FILE> <FUNCTION>` | file FIRST, function second |
| `taint` | `<FILE> <FUNCTION>` | not a directory |
| `complexity` | `<FILE> <FUNCTION>` | not a directory — but `cognitive`/`halstead`/`smells` ARE path-wide |
| `slice` | `<FILE> <FUNCTION> <LINE>` | — |
| `chop` | `<FILE> <FUNCTION> <FROM> <TO>` | needs **two** line numbers |

When unsure: `tldr <cmd> --help` — and note that `--help` does not surface the
`--symbol`⇒`--file` dependency, so prefer the forms above.

## Performance — `explain` scales with REPO SIZE, not the function

Nearly every command is fast; `explain` is the one exception, and its cost is
governed by how big the surrounding project is, not by the function you ask about.
Times below are order-of-magnitude:

| Command | Typical time | Notes |
|---|---|---|
| `structure`, `references`, `slice`, `complexity`, `taint`, `semantic`, `search` | **< 3 s** | fast — reach for these freely |
| `impact` | **~3 s** | reverse call graph |
| **`explain`** | **grows with the whole tree** | see the measured jump below |

**Root cause (verified by controlled test):** `explain` resolves the caller+callee
call graph across the ENTIRE detected project, so the *same function in the same
file* took **0.1 s alone in a 1-file dir vs 8.9 s inside a 395-file tree** — an ~90×
jump from repo size, nothing else changed. On a mid-size TypeScript repo a 3-line
function took **~300 s**. There is **no scoping flag** — `--project` / `--workspace`
/ `--no-workspace` are all rejected (`rc=2`); the only lever is the current working
directory (the auto-detected project root).

**So:** `explain` is fine on a small repo or a scoped checkout, and a trap on a large
one — do not open with it on an unfamiliar big codebase. It bundles signature +
purity + complexity + callers + callees, but you get the same facts in seconds by
composing the individually-fast commands: `tldr structure <file>` (signature) →
`tldr impact <fn> <path>` (callers) → `tldr references <fn> <path>`.

**The daemon (`tldr daemon start` + `tldr warm`) gave NO measurable speedup** in
testing — structural, `search`, and `semantic` commands ran the same cold or warmed,
and it did NOT help `explain` either (the cost is the graph computation, not a cold
cache). It is an index-reuse optimization whose payoff depends on repo size and query
volume; **measure before assuming it helps** rather than starting it reflexively. (It
never *hurts* correctness — it just may not pay off.)

## Full command catalog (63 commands; `[aliases]` shown)

Per-command flags & detail: run `tldr <cmd> --help`, or read `references/`.

**AST / structure (L1)**
- `tree` `[t]` — file tree structure
- `structure` `[s]` — functions, classes, imports per file, **plus anonymous callbacks**
  (see *Anonymous callbacks* below)
- `extract` `[e]` — complete module info for one file
- `imports` — parse import statements from a file
- `importers` — files that import a given module

**Call graph (L2)**
- `calls` `[c]` — cross-file call graph
- `impact` `[i]` — reverse call graph: who calls this function
- `dead` `[d]` — dead / unreachable code
- `hubs` — hub functions via centrality analysis
- `whatbreaks` `[wb]` — what breaks if a target changes
- `references` `[refs]` — all references to a symbol
- `deps` `[dep]` — module dependency analysis (import-level)

**Data flow (L3–L4)**
- `reaching-defs` `[rd]` — reaching definitions for a function
- `available` `[av]` — available expressions (CSE detection)
- `dead-stores` `[ds]` — dead stores (SSA-based)

**Program dependence / slicing (L5)**
- `slice` — backward program slice (only the lines affecting a target line)
- `chop` `[chp]` — chop slice (forward ∩ backward)
- `taint` `[ta]` — taint flow analysis (also a security command)

**Security**
- `secure` `[sec]` — security dashboard (taint, resources, bounds, contracts, behavioral, mutability)
- `vuln` — vulnerability scan (SQL injection, XSS, command injection)
- `api-check` `[ac]` — API misuse (missing timeouts, bare except, weak crypto, unclosed files)
- `resources` `[res]` — resource lifecycle (leaks, double-close, use-after-close)

**Quality & metrics**
- `smells` — code smells
- `complexity` — cyclomatic complexity per function
- `cognitive` `[cog]` — cognitive complexity (SonarQube algorithm)
- `halstead` `[hal]` — Halstead metrics per function
- `loc` — lines of code (code/comments/blanks)
- `churn` — git-based code churn
- `debt` — technical debt (SQALE)
- `health` `[h]` — comprehensive health dashboard
- `hotspots` `[hot]` — churn × complexity hotspots
- `clones` `[cl]` — code clone detection
- `cohesion` `[coh]` — class cohesion (LCOM4)
- `coupling` `[coup]` — afferent/efferent coupling + instability (call-edge based; use `deps`/`imports` for import-level)
- `coverage` `[cov]` — parse coverage reports (Cobertura XML, LCOV, coverage.py JSON)

**Patterns & architecture**
- `patterns` `[p]` — design pattern & convention detection
- `inheritance` `[inh]` — class inheritance hierarchies
- `surface` `[surf]` — machine-readable API surface of a library/package

**Contracts & verification**
- `contracts` `[con]` — infer pre/postconditions from guards/assertions/isinstance
- `specs` `[sp]` — extract behavioral specs from pytest test files
- `invariants` `[inv]` — infer invariants from test traces (Daikon-lite)
- `verify` `[ver]` — aggregated verification dashboard
- `interface` `[iface]` — interface contracts (public API signatures)
- `temporal` `[tem]` — mine temporal constraints (method call sequences)

**Search & context**
- `search` — enriched BM25 search with function-level context cards
- `semantic` `[sem]` * — natural-language code search
- `similar` `[sim]` * — find similar code fragments
- `dice` — similarity between two code fragments
- `context` — LLM-ready context from an entry point
- `definition` `[def]` — go-to-definition: where a symbol is defined
- `explain` `[exp]` — full function analysis (signature, purity, complexity, callers, callees)

**Aggregated / change**
- `todo` — aggregate improvement suggestions (dead code, complexity, cohesion, similar)
- `diff` `[df]` — AST-aware structural diff between two files
- `fix` `[fx]` — diagnose & auto-fix errors from compiler/runtime output
- `bugbot` — automated bug detection on code changes
- `change-impact` `[ci]` — find tests affected by code changes

**Diagnostics**
- `diagnostics` `[diag]` — type checking + linting
- `doctor` `[doc]` — check / install diagnostic tools (`tldr doctor --install python`)

**Daemon / cache / stats**
- `daemon` — daemon management (`start`, `stop`, `status`)
- `cache` — cache management (`stats`, `clear`)
- `warm` `[w]` — pre-warm the call-graph cache (see Performance — measure before relying on it)
- `stats` — tldr usage statistics
- `embed` `[emb]` * — generate embeddings for code chunks

\* `semantic`, `similar`, `embed` require the `semantic` build feature
(`cargo install tldr-cli --features semantic`). The first semantic run downloads
the arctic-embed-m model (~110 MB, cached).

### Anonymous callbacks

`structure` also emits a definition with `kind: "call"` for a **multi-line anonymous
callable passed to a call** — the test block, the `spawn`/`forEach`/`HandleFunc`
closure, the `do` block. Without this, the bodies that hold most of a test suite's
and most async code's real logic are invisible to `structure`, so an agent reads the
whole file to find them.

Covered in all 17 of the 18 supported languages that have such a form (C has none):
arrow functions and function expressions, Python `lambda`, Ruby/Elixir `do` blocks,
Go `func` literals, Rust closures, Java/Scala/C++/C# lambdas, Kotlin/Swift trailing
lambdas, PHP anonymous + arrow functions, Lua/Luau function definitions, OCaml `fun`.

**The name is the call that receives it**, because an anonymous callable has none of
its own:

| Source | Emitted name |
|---|---|
| `suiteSetup(async function () { … })` | `suiteSetup` |
| `test('a title here', async () => { … })` | `test:a-title-here` |
| `it 'does a thing' do … end` | `it:does-a-thing` |
| `http.HandleFunc("/x", func(w, r) { … })` | `HandleFunc:x` |
| two `test(…)` blocks with the same title | `test:same#1`, `test:same#2` |

A first string-literal argument becomes a `:slug` (lowercased, hyphenated, capped at
40 chars) so sibling blocks are distinguishable — and stable when a sibling is
inserted above them, which a positional index would not be. Only genuine collisions
fall back to the `#N` suffix.

Why it matters for an agent: `tldr structure spec/api_spec.rb` now returns one line
per `it` block with its line range, so `tldr slice`, `fastedit --replace`, and a
plain ranged `Read` can all target a single test without touching the file around it.

## Global flags & output formats

```
--format json      # default — structured, machine-readable
--format text      # human-readable (best for quick reading)
--format compact   # minified JSON for piping
--format sarif      # GitHub / VS Code integration
--format dot       # Graphviz visualization (call graphs, inheritance)
```

JSON is the default and is the most token-efficient for downstream parsing; use
`--format text` when you just want to read the answer.

## Daemon mode (fast repeated queries)

For more than a couple of queries on the same tree, run the in-memory daemon —
subsequent commands become cache hits:

```bash
tldr daemon start
tldr warm src/          # pre-warm the call-graph cache
tldr impact foo src/    # fast — served from the daemon
tldr cache stats
tldr daemon stop
```

The daemon replaces the old file-cache model (`.claude/cache/tldr/*.json`); state
is in memory, queried automatically by the CLI when the daemon is running.

## Editing code with fastedit (the WRITE companion)

`tldr` is READ-ONLY. To CHANGE code, use **`fastedit`** — an AST-aware editor (it
uses `tldr-code` internally, so tldr must be installed first). It finds the target
by SYMBOL NAME via tree-sitter, so you write ONLY the change (plus a line or two of
context) — never the old code repeated back. ~74% of edits resolve
deterministically (0 tokens, <1 ms); a local 1.7B model merges the rest (~40 tok).
Same discipline as tldr: invoke it INTENTIONALLY, by symbol.

Three edit modes: `--after <symbol>` = text insert after it (0 tok, instant) ·
`--replace <symbol>` deterministic = context anchors splice new lines (0 tok) ·
`--replace <symbol>` model = the 1.7B SLM merges your snippet into the ~35-line body.

| Need | Command |
|---|---|
| File's symbols + line ranges | `fastedit read <file>` |
| Replace a function/class body | `fastedit edit <file> --replace <symbol> --snippet '<body; #... keeps untouched lines>'` |
| Insert code after a symbol | `fastedit edit <file> --after <symbol> --snippet '<code>'` |
| Many edits to one file | `fastedit batch-edit <file> --edits '[{"after":"x","snippet":"…"}]'` |
| Edits across MANY files (one pass) | `fastedit multi-edit --file-edits '[{"file_path":"a.py","edits":[…]}]'` (`-` for stdin) |
| Find a symbol / its references | `fastedit search <query> [path]` (`--mode search\|regex\|hybrid\|references`) |
| Delete a symbol (caller-safe) | `fastedit delete <file> <symbol>` (refuses if cross-file callers; `--force`) |
| Move a symbol within a file | `fastedit move <file> <symbol> --after <other>` |
| Rename in one file (AST-verified) | `fastedit rename <file> <old> <new>` (`--dry-run`; skips strings/comments) |
| Rename across a tree | `fastedit rename-all <dir> <old> <new>` (`--dry-run`, `--only function`) |
| Move a symbol to another file (+rewrite importers) | `fastedit move-to-file <symbol> <src> <dst>` (`--dry-run`) |
| Verify / revert last edit | `fastedit diff <file>` · `fastedit undo <file>` |
| Diagnose setup | `fastedit doctor` |

Intentional read → check → edit loop:
```bash
fastedit read src/app.py                    # learn exact symbol names first
tldr impact handle_request src/             # blast radius before changing it
fastedit edit src/app.py --replace handle_request --snippet '
    validate(data)
    #...                                     # #... preserves the rest of the body
    logger.info("done")
'
fastedit diff src/app.py                     # confirm  ·  fastedit undo <file> to revert
```
`--replace` auto-preserves the signature; `#...` means "keep the untouched lines".
Prefer `fastedit rename`/`rename-all` over manual find-replace (AST-verified, skips
strings/comments). 13 languages (Python, JS, TS, Rust, Go, Java, C, C++, Ruby,
Swift, Kotlin, C#, PHP). Backend: local MLX (Apple Silicon) / vLLM (GPU), or any
OpenAI-compatible server via `FASTEDIT_BACKEND=llm` + `FASTEDIT_LLM_API_BASE=<url>`.
An optional MCP server (`fastedit-mcp`, 12 tools) + an Edit→fast_edit hook
(`fastedit-hook`) exist but are NOT enabled here: intentional-CLI use keeps the
per-turn token cost at zero, whereas a hook that fires on every tool call injects
text into the transcript and re-bills the cached prefix.

## Coexistence with distill

- **distill** is a generic, non-discriminating output-compression pipe. `tldr` is
  a deliberate instrument — it does not duplicate or replace distill, and it is
  not a Read-interceptor.
- An MCP server (`tldr-mcp`) is available for tool-style access — see
  `references/mcp-integration.md`.

## Per-command reference

`references/` holds the verbatim upstream docs — read the one for the category
you need:

- `references/command-overview.md` — the full README catalog
- `references/ast.md`, `callgraph.md`, `dataflow.md`, `metrics.md`,
  `patterns.md`, `quality.md`, `search.md`, `security.md`, `tools.md`,
  `daemon.md` — per-category command detail
- `references/mcp-integration.md` — using the `tldr-mcp` server

When the exact arguments of a command matter, prefer `tldr <cmd> --help` (ground
truth) over memory.
