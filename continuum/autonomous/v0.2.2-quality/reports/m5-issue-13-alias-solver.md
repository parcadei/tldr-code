# M5 — VAL-005: Alias field-store source propagation (issue #13)

## Summary

Closes [parcadei/tldr-code#13][issue]. Alias analysis silently dropped points-to
updates through field stores when the source variable later gained new points-to
info. Andersen's points-to soundness for the constraint
`base.field = source` requires `pts(loc.field) ⊇ pts(source)` for every
`loc ∈ pts(base)`. The solver re-evaluated the field store only when
`pts(base)` grew — never when `pts(source)` grew — leaving heap field
locations empty in any worklist ordering where the source variable's
points-to set arrived after the base was processed (e.g. through a phi
target).

## Files modified (1 source file + tests are inline)

- `crates/tldr-core/src/alias/solver.rs` — added `reverse_field_stores`
  index, populated in `index_constraints` for `Constraint::FieldStore`,
  and added the source-triggered re-propagation branch in
  `propagate_variable`. Two reproduction tests
  (`test_field_store_simple_no_phi`,
  `test_field_store_source_propagation_through_phi`) live in the inline
  `#[cfg(test)] mod tests`.

## Test status pre-fix

The two test names named in the issue body
(`test_field_store_simple_no_phi`,
`test_field_store_source_propagation_through_phi`) did **not** exist in
the codebase. `grep -rn` across `crates/tldr-core/` returned zero matches
on HEAD `88ddac6`. They were therefore written from scratch per the issue
body's bug description and the recommended-fix requirement that
`pts(param_obj.field)` contain the source variable's pointees.

## RED capture (HEAD `88ddac6`)

```
running 2 tests
test alias::solver::tests::test_field_store_simple_no_phi ... FAILED
test alias::solver::tests::test_field_store_source_propagation_through_phi ... FAILED

failures:

---- alias::solver::tests::test_field_store_simple_no_phi stdout ----
thread 'alias::solver::tests::test_field_store_simple_no_phi' panicked at crates/tldr-core/src/alias/solver.rs:899:9:
expected points-to set for param_obj.field to contain {param_a}; got: {} (alias-set mismatch — VAL-005/issue #13: source-propagation through field store missing)

---- alias::solver::tests::test_field_store_source_propagation_through_phi stdout ----
thread 'alias::solver::tests::test_field_store_source_propagation_through_phi' panicked at crates/tldr-core/src/alias/solver.rs:959:9:
expected points-to set for param_obj.field to contain alloc_3; got: {} (alias-set mismatch — VAL-005/issue #13: source-propagation through phi missing)

failures:
    alias::solver::tests::test_field_store_simple_no_phi
    alias::solver::tests::test_field_store_source_propagation_through_phi

test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 5059 filtered out
```

The literal phrase `alias-set mismatch` is the RED-REASON GATE evidence
required by VAL-005: both tests fail with concrete mismatch between the
expected and actual points-to sets for the heap field location
`param_obj.field`.

## GREEN capture (post-fix)

```
running 2 tests
test alias::solver::tests::test_field_store_source_propagation_through_phi ... ok
test alias::solver::tests::test_field_store_simple_no_phi ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 5059 filtered out
```

## Full alias suite

```
cargo test -p tldr-core --lib alias::
test result: ok. 127 passed; 0 failed; 34 ignored; 0 measured; 4900 filtered out
```

All 127 alias tests pass (34 pre-existing `#[ignore]` tests untouched;
the new two are the only additions).

## Matrix verification

```
cargo test -p tldr-cli --test exhaustive_matrix --features semantic --release -- --test-threads=1
test result: ok. 730 passed; 0 failed; 0 ignored; 0 measured

cargo test -p tldr-cli --test language_command_matrix --features semantic --release
test result: ok. 234 passed; 0 failed; 0 ignored; 0 measured
```

Combined: **964/964**. Note the exhaustive matrix is run single-threaded
to avoid the transient filesystem race on the cold fastembed model cache documented in the M4
prompt (multi-threaded run shows 676/730 with the 54 transient failures
that pass on subsequent or single-threaded runs).

## Clippy

```
cargo clippy --workspace --all-features --tests -- -D warnings
Finished `dev` profile in 26.59s
```

Clean.

## Fix details

### 1. New field on `AliasSolver`

```rust
/// Reverse field-store index: source -> [(base, field)].
reverse_field_stores: HashMap<String, Vec<(String, String)>>,
```

### 2. Population in `index_constraints` for `Constraint::FieldStore`

The previous code added an entry to `reverse_copy[source] = [base]`
("Track reverse for both base and source") but `propagate_variable`
never used that mapping for field stores — it only checked whether
`reverse_copy` targets were field-load targets or copy targets. Replaced
the spurious `reverse_copy` insertion with a dedicated index keyed by
`(base, field)`:

```rust
self.reverse_field_stores
    .entry(source.clone())
    .or_default()
    .push((base.clone(), field.clone()));
```

### 3. New source-triggered branch in `propagate_variable`

```rust
if let Some(stores) = self.reverse_field_stores.get(var).cloned() {
    for (base, field) in stores {
        let base_pts = self.points_to.get(&base).cloned().unwrap_or_default();
        self.propagate_field_store(&base_pts, &field, var);
    }
}
```

This restores Andersen's inclusion `pts(loc.field) ⊇ pts(source)` for
the case where `pts(source)` grows after `pts(base)` has already been
processed.

## Disjointness

Touched only `crates/tldr-core/src/alias/solver.rs`. No overlap with
M4 (`crates/tldr-daemon/src/state.rs`) or M6
(`crates/tldr-cli/src/commands/daemon/`).

## Commit

`c82e004` — see updated contract.json VAL-005.

[issue]: https://github.com/parcadei/tldr-code/issues/13
