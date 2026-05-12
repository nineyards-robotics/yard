`yard` is a standalone cli tool that helps setup and manage ROS2 workspaces. Yard's core concept is that it is an orchestrator above a curated set of tools and configurations that make ROS 2 development nice - yard does not hide any underlying tools. This should show up in the user being able to follow a completely non-yard related tutorial about a "standard" ROS workspace (including changing configurations or running commands). Yard has 3 main goals:

1. curate a set of software tools for robotics workspace development and manage their installation
2. configure those tools to work together seamlessly
3. provide common "shortcut" named verbs to common workflows (note the orchestration transparency - yard should display the commands its running, and the user should be able to replicate the same outcome by simply running the same commands)

## Language

yard is implemented in **Rust**. The deciding factor is the config-file mechanic (see below): yard round-trips shared TOML files like `pixi.toml` and `pyproject.toml` and must preserve user-authored comments, whitespace, and key order through every edit. Rust's `toml_edit` is purpose-built for format-preserving edits — it parses TOML into a syntax tree where mutations leave the surrounding text untouched. Go's mainstream TOML libraries decode/encode through structs and lose all of that, and writing a comment-preserving layer in Go would be substantial infrastructure work. Rust also gives us a single static binary with no runtime, which avoids Python-environment collisions in users' ROS workspaces.

## Distribution and version sync

yard is a **standalone binary**. It cannot be installed via any environment manager (pixi, nix, conda, devcontainer, ...) because yard *manages* those environments — installing yard inside one would be circular. yard sits at the same tier as `pixi`, `mise`, `rustup`, `direnv`: a tool that bootstraps a workspace and is itself bootstrapped independently.

### Installation

The primary install channel is a single-source script that fetches a pre-built binary from GitHub releases:

```bash
curl -fsSL https://yard.sh/install | sh                           # latest
curl -fsSL https://yard.sh/install | sh -s -- --version 0.5.2     # pinned to a specific version
YARD_VERSION=0.5.2 curl -fsSL https://yard.sh/install | sh        # via env var
```

### The `.yard/` folder

Every yard workspace has a `.yard/` folder for yard's own state. v1 contents:

```
.yard/
  state.toml      ← committed: records what yard last did to this workspace
```

```toml
# .yard/state.toml
last_applied_version = "0.5.2"
last_applied_at      = "2026-05-10T12:34:56Z"
```

The folder is committed to git so teams stay synchronized. It leaves room for non-committed caches later (`.yard/cache/`, gitignored).

### Warn on version mismatch

On every `yard apply`:

1. yard reads `.yard/state.toml` and compares `last_applied_version` to the running binary's version.
2. If they differ, yard warns: *"yard 0.6.0 is running, but `.yard/state.toml` says last apply was 0.5.2 - output may differ. To match, install 0.5.2 with: `curl -fsSL https://yard.sh/install | sh -s -- --version 0.5.2`."*
3. The apply proceeds anyway with the running binary's logic.
4. On success, yard writes its own version into `.yard/state.toml`.

This is **informational, not blocking**. Teams coordinate version upgrades by one member upgrading their binary, running apply, and committing the new `state.toml`. If we later find teams want a hard enforced pin, that's an additive change: a `yard_version` field in `yard.toml` that errors on mismatch.

## Architecture overview

```
yard.toml                      ← input: user's declaration of intent
   │
   ▼
yard core                      ← deserialize, validate
   │
   ▼
Modules                        ← each reads config, emits typed Contributions
   │
   ▼  (grouped by target adaptor, then merged)
Adaptors                       ← per config-type
   │   reconcile typed Desired against existing file contents
   ▼
Managed output files           ← pixi.toml, .gitignore, .pre-commit-config.yaml, ...
```

All of yard's logic and opinions ship in the binary. Cross-team consistency is achieved by everyone running the same yard binary version, with the version-mismatch warning above flagging drift.

Inside the binary, opinions are organized into **modules**. Modules read the parsed `yard.toml` and emit typed contributions; the engine groups contributions per target adaptor, merges them, and hands each adaptor a typed `Desired`. The module abstraction is invisible to the user — they see only user facing configuration sections in `yard.toml`. See the Modules section below.

Adaptors are independent of modules: each adaptor is keyed on the config type of the file it manages, accepts a typed `Desired`, and reconciles it against the existing file. Adding a new managed file type is purely a new adaptor implementation; adding a new opinion is purely a new module.

## Workspace declaration: `yard.toml`

`yard.toml` is yard's **input file**, not a yard-managed output. It is the user's declaration of what they want their workspace to be. yard creates a starter `yard.toml` during `yard init` and reads it on every subsequent run. yard does **not** round-trip-manage `yard.toml` with `# yard:managed` comments — the user owns it.

### Smart defaults

The guiding principle: a minimal `yard.toml` should produce a well-setup, working workspace. Users add keys only when they want to deviate from a module's defaults. The smallest sensible file declares only the required core values:

```toml
ros_distro = "jazzy"
```

### Schema

`yard.toml` deserializes into a single `YardConfig` struct, with `#[serde(deny_unknown_fields)]` at every level — an unrecognised top-level key is an error, ideally with a "did you mean X?" hint. This is the only typo-detection mechanism: there is no fuzzy matching at runtime.

Concrete v1 fields are listed in the v1 scope section. New fields **always** default when absent — yard never auto-rewrites `yard.toml` to add fields the user didn't ask for. If the user wants non-default behaviour on a new field, they add it from documentation.

### When yard writes `yard.toml`

Only on explicit user action:

- `yard init` writes the starter file.
- Future imperative verbs may write to it. These are user-invoked and obvious, never reconciliation-driven.

The reconciliation engine never touches `yard.toml`.

## Modules

Yard's opinions are organized internally into **modules**. A module is essentially a pure function from a `ModuleContext` (the parsed config plus runtime info about this invocation) to a list of typed contributions:

```rust
struct Module {
    id: &'static str,                                      // diagnostics only
    contribute: fn(&ModuleContext) -> Vec<Contribution>,
}

struct ModuleContext<'a> {
    config: &'a YardConfig,
    runtime: &'a RuntimeContext<'a>,
}
```

Modules are invisible to the user — there is no `[modules.*]` section in `yard.toml`. Users see only top-level configuration sections; each module reads whatever fields it cares about from the deserialized config and decides what to emit. The trade-off (vs. an explicit module abstraction in `yard.toml`) is that "what's available to configure" is a documentation concern, not something the user discovers from the file shape.

### Always-on, always-emit

Every module runs on every `yard apply`. There is no engine-level enable/disable. A module that "doesn't apply" simply returns an empty `Vec<Contribution>` — for example, the pre-commit module returns nothing if `pre_commit = false` (or absent). This keeps the trait surface minimal: the registry is just an ordered slice of `Module` values in the binary.

### Contributions

A `Contribution` is a typed fragment targeting a specific adaptor:

```rust
enum Contribution {
    Pixi(PixiContribution),
    PreCommit(PreCommitContribution),
    Gitignore(GitignoreContribution),
    // new variants as new adaptors land
}
```

Each `*Contribution` type is **additive**: a `PixiContribution` carries the deps, env vars, tasks, etc. that the contributing module wants in `pixi.toml`. Modules emit fragments; they never construct full `Desired` values.

### Merge

For each adaptor, the engine collects every contribution targeting it and merges them into the adaptor's `Desired`:

- **Maps and lists** (deps, env vars, ignore lines, hook IDs) union.
- **Scalars** (Python version, single-valued settings) error if two modules disagree, naming both modules and the offending key. Conflicts are loud, not silent.

Every adaptor runs on every apply, regardless of contribution count. With contributions, the adaptor reconciles them; with none, it runs against an empty `Desired` — which is what drives *removal* (see *Update lifecycle* below) of keys and blocks yard previously wrote but no longer wants. If both the merged `Desired` and the existing file are empty (no contributions and no on-disk file), the adaptor returns empty contents and the engine writes nothing — so the set of managed files is still dynamic, but removal is now symmetric with creation.

### Ordering

The module registry is an ordered slice. Order is fixed by the binary, not by `yard.toml`. Iteration order does not affect semantics (additive merges are commutative; scalar conflicts error rather than last-writer-wins) but does fix the order of items in the merged `Desired` for deterministic diffs.

### Future: user-authored modules

For v1, all modules are compiled into the binary. User-authored or remotely-loaded modules are an additive future feature — the engine sees no difference between a built-in module and a dynamically-loaded one, since both reduce to the same `fn(&YardConfig) -> Vec<Contribution>` shape.

## Configuration file management

The core mechanic of yard is the management of **shared output files** — `pixi.toml`, `.gitignore`, `.pre-commit-config.yaml`, etc. These files are co-edited by yard and the user. The mechanic must:

1. Make it obvious which parts a human reader can edit and which parts yard owns.
2. Detect user overrides and respect them.
3. Allow yard's defaults to evolve across preset versions without trampling user changes.
4. Operate at a *semantic* level — yard's core thinks "set the Python version to 3.11", not "edit line 42".

### Adaptors

Each kind of output file has an **adaptor**. An adaptor is keyed on the *config type*, not just the file format: `pixi.toml` and `pyproject.toml` are both TOML but get separate adaptors because their semantic schemas and the set of yard-managed keys are completely different.

Each adaptor implements a single reconciler interface:

```rust
trait ConfigAdaptor {
    /// Strongly-typed semantic intent built from the preset
    /// (e.g. PixiTomlDesired { python: "3.11", deps: [...], ... }).
    type Desired;

    /// Where this file lives in the workspace.
    fn path(&self, workspace: &Path) -> PathBuf;

    /// Produce planned file contents and a per-key action report.
    /// `existing` is None on first creation; Some(content) on every later run.
    /// User-authored content outside yard-managed regions is preserved verbatim.
    /// The engine inspects `actions` *before* writing: if any action is
    /// `Conflict`, the entire apply is blocked across all adaptors and
    /// nothing is written.
    fn apply(&self, desired: &Self::Desired, existing: Option<&str>) -> ApplyOutcome;
}

struct ApplyOutcome {
    contents: String,
    actions: Vec<KeyAction>,
}

enum KeyAction {
    InSync     { key: String },
    Updated    { key: String, from: Value, to: Value },
    Reemitted  { key: String, to: Value },
    Overridden { key: String },                                 // user explicitly took ownership via `# yard:overridden`; yard never touches
    Omitted    { key: String },                                 // user wrote `# yard:omit <key>`; yard does not re-emit
    Conflict   { key: String, on_disk: Value, default: Value }, // marker says managed, value differs from `default=` — apply blocks until resolved
    Deleted    { key: String, was: Value },                     // yard owned it, no longer wants it; key removed
}
```

There is no separate `update` operation. `apply(desired, None)` covers creation; `apply(desired, Some(content))` covers every later run. The adaptor owns all merge logic — yard's core never touches the file's syntax tree.

**Apply is atomic.** Every adaptor runs in a planning-only mode and returns its planned `contents` plus `actions`. The engine collects every action across every adaptor and inspects them as one batch. If any are `Conflict`, the engine surfaces all conflicts to the user, exits non-zero, and writes nothing — no half-applied state. Only if the batch is conflict-free does the engine commit each adaptor's planned contents to disk. The action report otherwise drives what `yard apply` prints (updated keys, deleted keys, etc.).

### Marking strategies

Two marking strategies cover the v1 file types. Each adaptor picks one based on the file format.

**Per-key comments** — for structured files where keys can be individually managed (TOML, YAML, JSONC):

```toml
python = "3.11"  # yard:managed default="3.11"
```

The trailing comment carries:

- `yard:managed` — declares ownership.
- `default="..."` — records the value yard last wrote. Self-documents what yard would set the key to if it were in control.

**Hard invariant: yard always writes the value and the comment together. The user only writes the value.** This is what makes conflict detection robust. If the user wants to take a key over permanently they rewrite the marker as `# yard:overridden` (see below); if they want yard to leave the key alone for this apply they revert the value to what `default=` records.

Marking only ever attaches to a single key. yard does not claim ownership of entire tables, sections, or arrays-of-tables in a structured file — those are user territory. If a module needs a collection of values managed (e.g. a map of conda dependencies), each entry is its own per-key managed line.

**Array-valued keys** are a refinement of the same scheme: per-key marking applied at the element level, so users can append items without forfeiting yard's management of the rest. When a managed key holds an array (e.g. `channels`, `platforms`), `default=` records the array yard last wrote — serialized — and yard's territory is the *set of elements* in that array rather than the whole value:

```toml
channels = ["conda-forge", "robostack-jazzy", "my-channel"]  # yard:managed default=["conda-forge","robostack-jazzy"]
```

On apply, the adaptor diffs the on-disk array against the recorded default:

- elements present in both → yard's, re-asserted in place.
- elements on disk but not in `default=` → user-added, preserved.
- elements in `default=` but missing from disk → user removed one of yard's items → `Conflict` (same as a divergent scalar). To genuinely drop one of yard's items, the user rewrites the whole key's marker to `# yard:overridden` (or uses `yard:omit`).

The rewritten array is `desired ∪ user-additions`; the new `default=` records just `desired`. Order is canonical: desired items first in module-registry order, user-added items after in the order they appeared on disk.

This is the only place `default=` records a set rather than a single value. The mental model is unchanged — yard's territory is re-asserted, the user's is preserved — only the unit of ownership is finer. Per-element opt-outs are deliberately not supported; to override a single element of a yard array, mark the whole key as overridden.

**Block fencing** — for unstructured / order-dependent text files (`.gitignore`, `.gitattributes`, `.envrc`):

```
# >>> yard:managed id=standard-ignores >>>
build/
install/
log/
# <<< yard:managed id=standard-ignores <<<
```

Every fence carries an `id=<slug>` — a user-readable name, unique within the file, that names what the block contains. Ids are unquoted and restricted to `[A-Za-z0-9_-]+` (no dots: fences are flat, dots are reserved for the dotted-key-path form `yard:omit` uses in structured files). The id is required on both the open and close markers and must match. A fence missing the id (or with mismatched open/close ids) is a parse error: the file fails loud rather than letting yard silently take or lose ownership. The id is what `yard:omit` and `yard:overridden` target when the user wants to opt out of one specific block in a file that may carry several.

yard owns everything inside the fence and rewrites the block on every apply. The user owns everything outside the fence — additional ignore rules above or below the block survive untouched. Per-line override inside the block is not supported (the block is all-or-nothing); to take over, the user converts the fence to `yard:overridden` (preserving the id).

### Classification

For per-key marking, the adaptor compares the actual value against the `default=` recorded in the comment and assigns each key a state:

| Marker / value on disk                            | State          | yard's plan for this `apply`                                                 |
|---------------------------------------------------|----------------|------------------------------------------------------------------------------|
| `yard:managed`, actual == `default=`              | in sync        | rewrite value + comment if the new desired differs; otherwise no-op          |
| `yard:managed`, actual != `default=`              | **conflict**   | report `Conflict`; the whole apply blocks until the user resolves            |
| `yard:overridden` (any value)                     | user-owned     | leave untouched; report `Overridden`                                         |
| `yard:omit <key>` present                         | omitted        | do not re-emit even if desired wants it; report `Omitted`                    |
| key absent, no `yard:omit`, adaptor wants it      | missing        | re-emit fresh; report `Reemitted`                                            |
| key present, marker missing, adaptor wants it     | unmarked       | attach `# yard:managed default=<desired>` — defer classification to the next apply |

The **conflict** row is the central rule. Yard never silently tolerates a `yard:managed` key whose value has drifted: the moment that happens, the next apply surfaces it. The user resolves it in one of two ways:

- **Revert** the value to what `default=` records. The key goes back to in-sync; yard rewrites to the current desired (which may differ from the recorded default) on the same or next apply.
- **Take it over** by rewriting the marker as `# yard:overridden`. Yard never touches it again unless the user rewinds the marker.

There is no third path. This is by design: keeping divergence as a hard signal is what lets the rest of the system stay simple — no soft-override state, no stale `default=` comments accumulating across versions, no quiet drift between the file and what yard would write.

The unmarked row is the symmetric case to the absent-key row. If the user strips just the `# yard:managed` comment but keeps the key, yard re-attaches the marker with `default=<desired>` and stops there for this apply — no same-run re-classification, no immediate flip to `Conflict`. Whatever divergence exists between the value and the freshly-recorded default surfaces on the next apply (or via a future `yard doctor` command that classifies without writing). Splitting it across two applies keeps each step doing one thing rather than racing to re-classify the moment the marker is attached.

This is also how `yard adopt` (a future command) works, essentially for free: taking control of an existing config system is the same operation — walk the file, find keys yard wants to manage, attach `# yard:managed default=<desired>`. Pre-existing values then surface as in-sync or `Conflict` on the next apply, where the user resolves per-key by reverting or overriding. The unmarked row and adopt are two entry points into the same code path.

Array-valued keys follow the same table at element granularity: yard's elements (those listed in `default=`) are classified individually. User-added elements (in actual but not `default=`) are silently preserved. Removal of any yard element (in `default=` but not actual) is a `Conflict` on the whole key.

For block fencing the comparison is coarser: yard rewrites the entire fence's interior on every apply. There is no per-line conflict — to take over, the user converts the fence to `yard:overridden`.

### `yard:overridden` and `yard:omit`

Two opt-outs let the user take explicit control.

**`yard:overridden`** is how the user takes a key over: yard never touches it again. The user converts a managed marker to overridden by rewriting the word `managed` as `overridden`. The marker carries **no `default=` payload**: yard has agreed never to write this key, so the historical default would only be dead weight and would rot as the binary upgrades. If the user later rewinds the marker back to `# yard:managed`, yard re-records `default=` from its current desired on the next apply.

```toml
python = "3.12"  # yard:overridden
```

This is also the canonical way to resolve a `Conflict`: yard surfaces "your value diverges from `default=`", and the user replies either by reverting the value (yard re-takes ownership) or by rewriting the marker as `yard:overridden` (user takes ownership).

For block-fenced files, the user takes a fence over by changing both markers, preserving the id:

```
# >>> yard:overridden id=standard-ignores >>>
... yard won't rewrite this block ...
# <<< yard:overridden id=standard-ignores <<<
```

**`yard:omit`** tells yard not to re-emit a managed key (or block) it would otherwise auto-create. The marker is a standalone comment line whose argument is matched against whatever target shape the file admits:

- In structured files (TOML, YAML, JSONC) the argument is a full dotted key path (e.g. `project.python`).
- In block-fenced files the argument is a fence id (e.g. `standard-ignores`).

A single charset covers both: `[A-Za-z0-9_.-]+`. The arg is looked up against the set of managed targets the adaptor knows about for this file (key paths or fence ids). If no target matches — typo, stale omit for a target yard no longer emits, dot in a gitignore omit — the adaptor warns and otherwise ignores the line. Unknown omits never block an apply.

```toml
# yard:omit project.python
```

```
# yard:omit standard-ignores
```

The marker may live anywhere in the file. v1 only supports the full-path form; a relative-to-section form (relying on toml_edit's comment-attachment) is feasible but deferred — full paths are unambiguous and easy to implement, and we can layer in relative paths as syntactic sugar later without breaking existing files.

### Update lifecycle

When the user installs a new yard version or updates `yard.toml` and runs `yard apply`, the engine:

1. Runs every module against the parsed `yard.toml` to produce `Contribution` fragments, then merges them per adaptor into a typed `Desired` (per the *Modules → Merge* rules).
2. Calls `adaptor.apply(desired, existing)` on every adaptor — *planning only*, nothing is written yet.
3. Each adaptor walks the existing file and classifies every managed key, array element, and fence per the table in *Classification*.
4. The engine collects all `KeyAction`s across all adaptors. If any are `Conflict`, the engine surfaces them, exits non-zero, and writes nothing — no file is modified this run.
5. If the batch is conflict-free, the engine commits each adaptor's planned contents. Per-key, the action depends on classification:
   - in-sync keys whose desired value equals the recorded `default=` are left as-is (`InSync`);
   - in-sync keys whose desired value has changed are rewritten with the new value *and* a refreshed `default=` (`Updated`);
   - keys absent from disk that the adaptor still wants are written fresh with `# yard:managed default=<desired>` attached (`Reemitted`);
   - keys present on disk without a marker that the adaptor wants get `# yard:managed default=<desired>` attached — classification stops there for this apply, and any value divergence surfaces on the next one;
   - `yard:overridden` keys are left untouched (`Overridden`);
   - `yard:omit` keys are skipped (`Omitted`);
   - keys yard no longer wants are deleted (`Deleted`), per *Removal* below.

   Array-valued keys are reconciled at element granularity: yard's elements (listed in `default=`) are re-asserted, user-added elements preserved, and the rewrite is `desired ∪ user-additions` with `default=` recording just `desired`. Block fences have their interior rewritten in full when in-sync, are left untouched when `yard:overridden`, and are removed when yard no longer emits them.
6. User-authored content (sections, keys, comments without a `yard:` prefix) is preserved verbatim throughout.

Re-running `yard apply` against the same configuration is also valid and is the natural way to repair a workspace where managed content has been accidentally deleted.

### Removal

Removal is the mirror of creation: when a module stops emitting a key (or block, or fence) that yard previously wrote, the next `apply` reconciles the disappearance.

- **In-sync at removal** (`actual == default=`) → yard deletes the key/block entirely and reports `Deleted`. The user accepted yard's ownership; yard is now relinquishing it cleanly.
- **Marker says `yard:managed` but value differs from `default=`** → `Conflict`, same as any other apply. Removal never gets to silently delete a divergent value; the user resolves the conflict first (revert → falls into the in-sync removal path on the next apply; or convert to `yard:overridden` → falls into the next bullet).
- **`yard:overridden` at removal** → no change, reported as `Overridden`. The user explicitly took the key over; yard has no claim left to remove.
- **`yard:omit` at removal** → no file change, reported as `Omitted`, and yard **emits a warning**. The omit already suppressed yard's emission, so there is no managed key on disk to act on; the omit marker itself is a user-authored comment line. Its target is no longer in the adaptor's managed set, which makes it a stale omit — yard warns (per the standard `yard:omit` rule above) so the user knows the marker is now redundant, and otherwise leaves the line untouched. Cleaning up the marker is the user's call.

Array-valued keys handle removal mechanically without a new action variant: items in the previous `default=` that drop out of the new desired simply aren't in `desired ∪ user-additions` on the rewrite, so they vanish. The whole-key actions above apply if the *entire* array is removed.

Block-fenced files remove whole fences (matched by id): in-sync fences are deleted (the open/close markers and everything between), reported as `Deleted` against the fence id. Overridden fences stay. There is no in-fence conflict state — fence interiors are all-or-nothing.

### Semantic layer

The `Desired` associated type per adaptor is the seam between yard's core model and file-level concerns. yard's core never constructs TOML/YAML/text directly; modules emit typed `Contribution` fragments, the engine merges them into a `Desired`, and the adaptor consumes that. This decouples the core from file formats and means adding a new managed file type is purely a new adaptor implementation.

## CLI verbs

For v1, two verbs share a single reconciliation engine:

- **`yard init`** — bootstraps a workspace. Refuses if `yard.toml` already exists. Writes a starter `yard.toml` (smart defaults), then runs the apply engine to produce the initial managed files.
- **`yard apply`** — the workhorse verb. Reads `yard.toml`, runs every module to produce contributions, merges per adaptor, runs every adaptor's `apply`, writes results. Used after binary upgrades, after `yard.toml` edits, for repairs, and for any subsequent reconciliation.

Migration from one yard version to the next is **not its own verb** — it is one reason to call `apply`. The engine doesn't care.

## v1 scope

**Input:**

- `yard.toml`

**yard-owned state:**

- `.yard/state.toml` — last-applied yard binary version, used for cross-team version-mismatch warnings.

**Managed output files (each created only if at least one module contributes):**

- `pixi.toml` — env management. Per-key comment marking.
- `.pre-commit-config.yaml` — dev-quality config. Per-key comment marking.
- `.gitignore` — block fencing.

**CLI verbs:**

- `yard init`, `yard apply`.

**Distribution:**

- Standalone binary via `curl | sh` install script with `--version` pin support. Pre-built binaries published to GitHub releases.
