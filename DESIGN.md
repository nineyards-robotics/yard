`yard` is a standalone cli tool that helps setup and manage ROS2 workspaces. Yard's core concept is that it is an orchestrator above a curated set of tools and configurations that make ROS 2 development nice - yard does not hide any underlying tools. This should show up in the user being able to follow a completely non-yard related tutorial about a "standard" ROS workspace (including changing configurations or running commands). Yard has 3 main goals:

1. curate a set of software tools for robotics workspace development and manage their installation
2. configure those tools to work together seamlessly
3. provide common "shortcut" named verbs to common workflows (note the orchestration transparency - yard should display the commands its running, and the user should be able to replicate the same outcome by simply running the same commands)

## Language

yard is implemented in **Rust**. The deciding factor is the config-file mechanic (see below): yard round-trips shared TOML files like `pixi.toml` and `pyproject.toml` and must preserve user-authored comments, whitespace, and key order through every edit. Rust's `toml_edit` is purpose-built for format-preserving edits — it parses TOML into a syntax tree where mutations leave the surrounding text untouched. Go's mainstream TOML libraries decode/encode through structs and lose all of that, and writing a comment-preserving layer in Go would be substantial infrastructure work. Rust also gives us a single static binary with no runtime, which avoids Python-environment collisions in users' ROS workspaces.

## Architecture overview

```
yard.toml                      ← input: user's declaration of intent
   │
   ▼
yard core
   │   loads the preset matching `preset_version`
   ▼
Preset                         ← versioned bundle of opinions, decoupled from binary
   │   yields typed Desired values
   ▼
Adaptors                       ← per config-type, compiled into binary
   │   reconcile Desired against existing file contents
   ▼
Managed output files           ← pixi.toml, .gitignore, .pre-commit-config.yaml, ...
```

The two halves of yard are deliberately separated:

- The **binary** holds the *mechanic*: adaptor logic, parsing, marker handling, file IO, the CLI.
- The **preset** holds the *opinions*: what Python version, what dependencies, what aliases. It is versioned independently and pinned in `yard.toml`. Two team members on different yard binaries see identical files as long as both binaries support the preset format and they reference the same `preset_version`.

For v1 the preset is compiled into the binary. The `Preset` type and its loader are designed so that remote loading (HTTP, git ref, OCI) is a purely additive future feature — the rest of yard sees no difference.

## Workspace declaration: `yard.toml`

`yard.toml` is yard's **input file**, not a yard-managed output. It is the user's declaration of what they want their workspace to be. yard creates a starter `yard.toml` during `yard init` and reads it on every subsequent run. yard does **not** round-trip-manage `yard.toml` with `# yard:managed` comments — the user owns it.

### Smart defaults

The guiding principle: a minimal `yard.toml` should produce a well-setup, working workspace. Users add keys only when they want to deviate from the preset's defaults. The smallest possible file is essentially just a preset version pin:

```toml
preset_version = "1"
```

### Schema (v1)

| Key              | Required | Default          | Notes                                                        |
|------------------|----------|------------------|--------------------------------------------------------------|
| `preset_version` | yes      | —                | Pins the preset version for team sync. No implicit default.  |
| `preset`         | no       | `"yard-default"` | Named preset. Reserved for future when multiple presets ship. |

Future fields (ROS distro, env manager choice, package list, etc.) will land here as the preset model gains them. New fields **always** default when absent — yard never auto-rewrites `yard.toml` to add fields the user didn't ask for. If the user wants non-default behaviour on a new field, they add it from documentation.

### When yard writes `yard.toml`

Only on explicit user action:

- `yard init` writes the starter file.
- Future imperative verbs (e.g. a hypothetical `yard preset bump`) may write to it. These are user-invoked and obvious, never reconciliation-driven.

The reconciliation engine never touches `yard.toml`.

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

    /// Produce new file contents and a per-key action report.
    /// `existing` is None on first creation; Some(content) on every later run.
    /// User-authored content outside yard-managed regions is preserved verbatim.
    fn apply(&self, desired: &Self::Desired, existing: Option<&str>) -> ApplyOutcome;
}

struct ApplyOutcome {
    contents: String,
    actions: Vec<KeyAction>,
}

enum KeyAction {
    InSync     { key: String },
    Updated    { key: String, from: Value, to: Value },
    Overridden { key: String, user_value: Value, default: Value },
    Frozen     { key: String },
    Reemitted  { key: String, to: Value },
    Omitted    { key: String },
}
```

There is no separate `update` operation. `apply(desired, None)` covers creation; `apply(desired, Some(content))` covers every later run. The adaptor owns all merge logic — yard's core never touches the file's syntax tree. The action report drives what `yard apply` prints to the user (overridden keys, updated keys, etc.).

### Marking strategies

Two marking strategies cover the v1 file types. Each adaptor picks one based on the file format.

**Per-key comments** — for structured files where keys can be individually managed (TOML, YAML, JSONC):

```toml
python = "3.11"  # yard:managed default="3.11"
```

The trailing comment carries:

- `yard:managed` — declares ownership.
- `default="..."` — records the value yard last wrote. Self-documents what yard would set the key to if it were in control.

**Hard invariant: yard always writes the value and the comment together. The user only writes the value.** This is what makes override detection robust.

For block-shaped managed content within a structured file (entire tables, lists), the same idea applies via a leading comment on the section:

```toml
# yard:managed
[tool.yard.bootstrap]
... contents owned by yard ...
```

**Block fencing** — for unstructured / order-dependent text files (`.gitignore`, `.gitattributes`, `.envrc`):

```
# >>> yard:managed >>>
build/
install/
log/
# <<< yard:managed <<<
```

yard owns everything inside the fence and rewrites the block on every apply. The user owns everything outside the fence — additional ignore rules above or below the block survive untouched. Per-line override detection inside the block is not supported (the block is all-or-nothing); to override, the user converts the fence to `yard:frozen`.

### Override detection

For per-key marking, the adaptor compares the actual value against the `default=` recorded in the comment:

| Actual vs. comment-default        | Meaning                          | yard's action on next `apply`                                       |
|-----------------------------------|----------------------------------|---------------------------------------------------------------------|
| equal                             | in sync                          | rewrite value + comment if the preset's desired default has changed |
| diverged                          | user override                    | leave the key alone; report as `Overridden`                         |
| key absent, but adaptor wants it  | section deleted or never created | re-emit unless a `yard:omit` marker is present                      |

The collision case — *the preset's new default happens to equal the user's override* — resolves cleanly. yard sees `actual = V, comment-default = old`, classifies as diverged (because the comment hasn't been updated to V), and leaves the key alone. The comment-default is now stale, but that doesn't break anything: yard never compares comment-default against the desired value, only against the actual value to detect divergence. If the user later wants yard to retake ownership, they delete their override and the next `apply` re-emits with the current default.

For block fencing the comparison is simpler: yard hashes the block's expected content; if the on-disk block matches a previously-emitted hash, it's safe to rewrite, otherwise it's an override.

### `yard:frozen` and `yard:omit`

Two opt-outs let the user take stronger control:

```toml
python = "3.12"  # yard:frozen default="3.11"
```

`yard:frozen` tells yard never to touch this key, regardless of divergence. Useful for permanent overrides.

```toml
# yard:omit
# [tool.something]   ← managed section the user removed on purpose
```

`yard:omit` tells yard not to re-emit a managed key or section it would otherwise auto-create.

For block-fenced files the same markers apply at the fence:

```
# yard:frozen >>>
... yard won't rewrite this block ...
# yard:frozen <<<
```

### Update lifecycle

When the user bumps `preset_version` and runs `yard apply`, the engine for each managed file:

1. Loads the new `Desired` from the new preset.
2. Calls `adaptor.apply(desired, existing)`.
3. The adaptor walks the existing file and classifies every managed key/block.
4. In-sync keys are rewritten with the new value *and* a new `default=` comment.
5. Overridden keys are left alone and reported as `Overridden`.
6. Frozen keys are never touched.
7. Missing keys are re-emitted unless `yard:omit` is present.
8. User-authored content (sections, keys, comments without a `yard:` prefix) is preserved verbatim.

Re-running `yard apply` against the same `preset_version` is also valid and is the natural way to repair a workspace where managed content has been accidentally deleted.

### Semantic layer

The `Desired` associated type per adaptor is the seam between yard's core model and file-level concerns. yard's core never constructs TOML/YAML/text directly; it builds typed `Desired` values from the preset and hands them to adaptors. This decouples the core from file formats and means adding a new managed file type is purely a new adaptor implementation.

## CLI verbs

For v1, two verbs share a single reconciliation engine:

- **`yard init`** — bootstraps a workspace. Refuses if `yard.toml` already exists. Writes a starter `yard.toml` (smart defaults), then runs the apply engine to produce the initial managed files.
- **`yard apply`** — the workhorse verb. Reads `yard.toml`, loads the preset, runs every adaptor's `apply`, writes results. Used for version bumps (after the user edits `preset_version`), repairs, and any subsequent reconciliation.

Migration from one preset version to the next is **not its own verb** — it is one reason to call `apply`. The engine doesn't care.

## v1 scope

**Input:**

- `yard.toml`

**Managed output files:**

- `pixi.toml` — env management. Per-key comment marking.
- `.pre-commit-config.yaml` — dev-quality config. Per-key comment marking.
- `.gitignore` — block fencing.

**CLI verbs:**

- `yard init`, `yard apply`.

Everything else (additional file types, package-level files like `package.xml`/`CMakeLists.txt`, status/dry-run verbs, remote presets, workflow shortcut verbs) is deferred until the v1 mechanic is solid.
