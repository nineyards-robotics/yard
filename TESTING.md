# Testing

yard's testing strategy is built around **fixture files on disk** as the unit of truth. A test scenario is a directory containing the inputs (real-shaped, as the code's caller would produce them) and the expected outputs (real-shaped, as the code under test would emit them). The test driver reads inputs, runs the code, and diffs the result against the expected files. When behaviour legitimately changes, regenerating the expected files is a one-flag operation — the diff lands in `git status` and gets reviewed like any other change.

This document is for maintainers adding tests. For the architectural rationale behind adaptors / modules / engine, see [`DESIGN.md`](DESIGN.md).

## Where does my test go?

| You're testing... | Where it lives | Driver |
|---|---|---|
| One adaptor's `apply` in isolation | `src/adaptors/<name>.rs` (`#[cfg(test)] mod tests`) + fixtures next door | `crate::adaptors::test_harness::run_apply_fixture` |
| One module's `contribute` in isolation | `src/modules/<name>.rs` (`#[cfg(test)] mod tests`) + fixtures next door | `crate::modules::test_harness::run_module_fixture` |
| The engine + modules + adaptors + filesystem, end to end | `tests/apply_<scenario>.rs` | shells out to the `yard` binary via `assert_cmd` |
| The CLI surface (flags, subcommand routing) | `tests/cli.rs` | shells out to the `yard` binary |
| `yard.toml` parsing | `tests/config.rs` + fixtures under `tests/fixtures/config/` | calls `YardConfig::from_path` directly |

**Rule of thumb:** if it's testing one piece of yard's logic in isolation, it goes co-located in `src/` and uses a harness. If it's testing the binary end-to-end as a user would invoke it, it goes in `tests/`.

## The universal helper: `assert_golden`

Lives in [`src/test_support.rs`](src/test_support.rs). The whole file is ~35 lines. Both harnesses (and any future test that wants to diff against a fixture file) call into it.

```rust
crate::test_support::assert_golden(&path, &actual_string);
```

Behaviour:

- **Default:** read `path`, `pretty_assertions::assert_eq!` against `actual_string`. On failure, prints a coloured diff and the fixture path.
- **`UPDATE_GOLDENS=1` set:** rewrite `path` with `actual_string` instead of asserting. Used to regenerate fixtures after an intentional behaviour change.

Workflow when a test fails because behaviour changed:

```bash
UPDATE_GOLDENS=1 cargo test
git diff   # review the regenerated fixtures
git add ...
```

Workflow when adding a brand-new fixture:

1. Create the directory and the input files.
2. Add the `#[test] fn` (one line).
3. Run `UPDATE_GOLDENS=1 cargo test <fixture_name>` to populate the expected files.
4. Read what was generated and confirm it's what the code *should* produce.
5. Commit.

Step 4 is the important one — `UPDATE_GOLDENS=1` will happily write whatever the code currently emits, including a bug. The diff in `git status` is your review.

## Adaptor tests

Adaptors are pure `(Desired, Option<&str>) -> ApplyOutcome` functions (see DESIGN.md for the contract). Their tests run them in isolation — no engine, no filesystem, no binary.

### Fixture layout

```
src/adaptors/<name>/fixtures/<scenario>/
  desired.ron                ← the typed Desired, serialized as RON
  existing.<ext>             ← optional: pre-existing file content. Absent ⇒ "no file yet"
  expected.<ext>             ← what apply() should produce as ApplyOutcome.contents
  expected.actions           ← one `Kind key` per line for ApplyOutcome.actions
```

Filename `<ext>` matches the real output file (`existing.gitignore`, `existing.pixi.toml`, etc.) so a reader can `cat` any fixture file and immediately see the real-shaped artifact.

Real example, fully populated:

```
src/adaptors/gitignore/fixtures/update_in_place/
  desired.ron
  existing.gitignore
  expected.gitignore
  expected.actions
```

### The harness

[`src/adaptors/test_harness.rs`](src/adaptors/test_harness.rs) provides:

- `ApplyHarness` — a const struct holding `fixtures_root`, `existing_filename`, `expected_filename` for one adaptor.
- `run_apply_fixture::<D, _>(&HARNESS, scenario, apply)` — does the read-input → call-apply → golden-diff loop.
- `format_actions(&[KeyAction]) -> String` — canonical serialization of the actions list.

The whole adaptor-test boilerplate looks like this (full real example from `src/adaptors/gitignore.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptors::test_harness::{ApplyHarness, run_apply_fixture};

    const HARNESS: ApplyHarness = ApplyHarness {
        fixtures_root: concat!(env!("CARGO_MANIFEST_DIR"), "/src/adaptors/gitignore/fixtures"),
        existing_filename: "existing.gitignore",
        expected_filename: "expected.gitignore",
    };

    fn run(name: &str) {
        run_apply_fixture::<GitignoreDesired, _>(&HARNESS, name, |d, e| {
            GitignoreAdaptor.apply(d, e)
        });
    }

    #[test] fn create_fresh()           { run("create_fresh"); }
    #[test] fn in_sync()                { run("in_sync"); }
    #[test] fn update_in_place()        { run("update_in_place"); }
    // ...
}
```

### Adding a new fixture (existing adaptor)

1. `mkdir src/adaptors/<name>/fixtures/<scenario>/`
2. Write `desired.ron`. The file should deserialize into the adaptor's `Desired` type. The named-struct form is preferred for documentation:
   ```ron
   GitignoreDesired(
       lines: ["build/", "install/"],
   )
   ```
3. (Optional) Write `existing.<ext>` if the scenario starts from an existing file. Omit it for fresh-create.
4. Add a one-liner test: `#[test] fn <scenario>() { run("<scenario>"); }`
5. `UPDATE_GOLDENS=1 cargo test <scenario>`
6. Eyeball the generated `expected.<ext>` and `expected.actions`. If correct, commit.

### Adding a new adaptor

1. Create `src/adaptors/<name>.rs` with the adaptor's `Desired`, `Contribution`, and the adaptor struct implementing `apply`. Derive `Deserialize` on `Desired` (so it can be loaded from `desired.ron`) and `Serialize` on `Contribution` (so it can be rendered into module test goldens).
2. Add `pub mod <name>;` to `src/adaptors.rs`.
3. Add a variant to `enum Contribution` in `src/lib.rs`.
4. Add the `#[cfg(test)] mod tests` block from the template above. Pick `existing_filename` / `expected_filename` to match the real on-disk file (e.g. `pixi.toml`).
5. Create `src/adaptors/<name>/fixtures/<scenario>/` directories per the section above.

### `expected.actions` format

The harness renders `Vec<KeyAction>` as one `Kind key` per line:

```
Reemitted .gitignore:managed
```

Inner payloads (`from`/`to` strings on `Updated`, `to` on `Reemitted`) are intentionally dropped — they're derivable from `expected.<ext>` and would just duplicate surface to maintain. If a future variant needs richer assertions, extend `format_actions` in the harness once and every adaptor benefits.

When you add a new `KeyAction` variant, add the corresponding match arm to `format_actions` in `src/adaptors/test_harness.rs`. That's the only test-related change required.

## Module tests

Modules are pure `fn(&YardConfig) -> Vec<Contribution>`. Their tests run them in isolation — no engine, no adaptors.

### Fixture layout

```
src/modules/<name>/fixtures/<scenario>/
  yard.toml                       ← the input, real user-shape
  expected.contributions.ron      ← serialized Vec<Contribution> the module emits
```

Real example:

```
src/modules/ros_workspace/fixtures/emits_standard_ignores/
  yard.toml
  expected.contributions.ron
```

`yard.toml` is the actual user-input shape (what the user would write). It's parsed via the public `YardConfig::from_str`. Output is RON because `Contribution` is a Rust enum and RON renders tagged unions naturally — `Gitignore(GitignoreContribution(lines: [...]))` reads like the source code.

### The harness

[`src/modules/test_harness.rs`](src/modules/test_harness.rs) provides:

- `ModuleHarness` — const struct holding `fixtures_root`.
- `run_module_fixture(&HARNESS, scenario, contribute)` — read `yard.toml` → call `contribute` → serialize → golden-diff.

Full real example from `src/modules/ros_workspace.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::test_harness::{ModuleHarness, run_module_fixture};

    const HARNESS: ModuleHarness = ModuleHarness {
        fixtures_root: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/modules/ros_workspace/fixtures"
        ),
    };

    fn run(name: &str) {
        run_module_fixture(&HARNESS, name, contribute);
    }

    #[test] fn emits_standard_ignores() { run("emits_standard_ignores"); }
}
```

### Adding a new fixture (existing module)

1. `mkdir src/modules/<name>/fixtures/<scenario>/`
2. Write `yard.toml` — a real user-input snippet that exercises the scenario you care about.
3. Add a one-liner test: `#[test] fn <scenario>() { run("<scenario>"); }`
4. `UPDATE_GOLDENS=1 cargo test <scenario>`
5. Read the generated `expected.contributions.ron`. Confirm the module emitted what it should. Commit.

### Adding a new module

1. Create `src/modules/<name>.rs` with a `pub fn contribute(&YardConfig) -> Vec<Contribution>`.
2. Add `pub mod <name>;` to `src/modules.rs`.
3. Register the module in the static `MODULES` registry in `src/modules.rs`.
4. Add the `#[cfg(test)] mod tests` block from the template above.
5. Create at least one fixture under `src/modules/<name>/fixtures/`.

## Integration tests (`tests/`)

The `tests/` directory holds tests that exercise yard end-to-end as a user would. Each `.rs` file in `tests/` compiles to a separate test binary; tests in here can only see yard's *public* API.

### What's there today

- **[`tests/cli.rs`](tests/cli.rs)** — pins down the CLI surface: `--help` lists the verbs, `--version` prints, unknown subcommand fails. Add a test here when you add or rename a verb.
- **[`tests/config.rs`](tests/config.rs)** — exercises `YardConfig::from_path` against fixtures under `tests/fixtures/config/`. Each fixture is a real-shape `yard.toml` that demonstrates an accepted or rejected input variant. Add a fixture + test here when you add a `yard.toml` field or a new validation rule.
- **[`tests/apply_gitignore.rs`](tests/apply_gitignore.rs)** — full-stack scenarios: spin up a tempdir, write a `yard.toml`, shell out to the `yard` binary via `assert_cmd`, inspect the resulting files. Catches everything that the unit-level harnesses can't: that the engine wires the adaptor to the right path, that a second apply is genuinely idempotent on disk, that user-authored content survives across runs, that the CLI prints the right summary lines.

### When to add an integration test

- **You added or renamed a CLI verb / flag** → extend `tests/cli.rs`.
- **You added or rejected a new `yard.toml` shape** → add a fixture under `tests/fixtures/config/` and a test in `tests/config.rs`.
- **You added a new managed file type** → add a `tests/apply_<filetype>.rs` mirroring `apply_gitignore.rs`. The unit-level adaptor tests prove `apply` works in isolation; the integration test proves the engine drives it correctly through real I/O.
- **You hit a bug that wasn't caught by unit tests** → first ask whether the unit harness could be extended to catch it; if it genuinely needs the binary running against a real filesystem, it goes in `tests/`.

Integration tests are slower (each one rebuilds and reruns the binary in a tempdir), so prefer the unit-level harnesses when they suffice.

## Running tests

```bash
cargo test                              # everything: unit + integration + doc
cargo test --lib                        # unit tests only (the harnesses)
cargo test --test apply_gitignore       # one integration test binary
cargo test create_fresh                 # any test whose name contains "create_fresh"
UPDATE_GOLDENS=1 cargo test             # rewrite all golden fixtures
UPDATE_GOLDENS=1 cargo test in_sync     # regenerate only matching fixtures
```

## Format choices

A couple of decisions worth knowing about so future fixtures stay consistent:

- **Fixture inputs that mirror user files use the user format.** `tests/fixtures/config/*.toml` and `src/modules/*/fixtures/*/yard.toml` are real `yard.toml` shapes — what the user actually writes. Reading the fixture *is* reading documentation of "what input drives this code path."
- **Fixture inputs that are arbitrary Rust types use RON.** `desired.ron` deserializes directly into the adaptor's `Desired` struct. RON renders Rust enums and tuple variants naturally (`Gitignore(GitignoreContribution(...))`), supports comments, and round-trips Rust shapes more cleanly than TOML or JSON.
- **Expected output filenames mirror the real on-disk file.** `expected.gitignore` (not `expected.txt`) so a reader scanning a fixture directory sees three real-shaped files (`desired.ron`, `existing.gitignore`, `expected.gitignore`) and immediately groks the test.
- **Inline assertions are reserved for non-fixture-shaped tests.** `merges_and_deduplicates_lines` in `src/adaptors/gitignore.rs` is a pure-function unit test for `from_contributions` — different shape from `apply`, so it stays as a regular `#[test]` with `assert_eq!`. Don't force everything through the fixture harness; use it where it earns its keep.

## When the harness gets in your way

The harnesses are deliberately small (~50 lines each). If a new adaptor or module needs a different fixture shape, extending the harness is fine — but consider whether the new shape genuinely belongs alongside the existing one or wants its own helper. The bar is "does every adaptor / module benefit from this change."

For one-off testing patterns (e.g. property-based tests over a single function), don't fight the harness — write a regular `#[test]` next to the fixture-driven ones. The fixture pattern is a strong default, not a mandate.
