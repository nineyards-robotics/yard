# Yard marker system summary

Yard co-manages generated workspace files with users. Markers define exactly which parts of a file yard owns, which parts the user owns, and when an edit is a conflict.

## Per-key markers

Used in structured files such as TOML, YAML, or JSONC.

```toml
python = "3.11"  # yard:managed default="3.11"
```

Meaning:

- `yard:managed` says yard owns this key.
- `default=...` records the value yard last wrote.
- If the current value still equals `default`, yard may update it to the new desired value.
- If the current value differs from `default`, yard reports a conflict and blocks apply.

Yard manages individual keys, not whole tables or sections.

## Block markers

Used in plain text or order-sensitive files such as `.gitignore`.

```text
# >>> yard:managed id=standard-ignores >>>
build/
install/
log/
# <<< yard:managed id=standard-ignores <<<
```

Meaning:

- yard owns the whole fenced block;
- the `id` identifies the managed block;
- content outside the fence is user-owned;
- block overrides are all-or-nothing.

## User opt-outs

### `yard:overridden`

The user takes permanent ownership.

```toml
python = "3.12"  # yard:overridden
```

For fenced blocks, both fence markers become `yard:overridden` while keeping the same `id`.

Yard leaves overridden content untouched.

### `yard:omit`

The user tells yard not to create or re-create a managed item.

```toml
# yard:omit project.python
```

```text
# yard:omit standard-ignores
```

Unknown or stale omits warn but do not block apply.

## Arrays

Array keys are managed at element level.

```toml
channels = ["conda-forge", "robostack-jazzy", "custom"]  # yard:managed default=["conda-forge","robostack-jazzy"]
```

- Items listed in `default=` are yard-owned.
- Extra on-disk items are user additions and are preserved.
- Removing a yard-owned item is a conflict.
- Reordering yard-owned items relative to each other is a conflict.

### Ordering algorithm

Yard preserves user-chosen interleaving. User items are anchored to the yard item (or start-of-array) that immediately precedes them on disk.

Given disk `[X, A, Y, B, Z, C, W]` where yard items are `[A, B, C]`:

**1. Build an anchor map** from the on-disk array:

| Anchor | User items |
|--------|------------|
| START  | [X]        |
| A      | [Y]        |
| B      | [Z]        |
| C      | [W]        |

**2. Verify** that yard-owned items appear on disk in the expected relative order. If not, report a conflict.

**3. Reconstruct** by walking yard's new desired order, emitting each anchor's user items then the yard item:

- **Yard adds D between A and B** (desired `[A, D, B, C]`): Y stays anchored to A → `[X, A, Y, D, B, Z, C, W]`.
- **Yard removes B** (desired `[A, C]`): re-anchor B's user items to B's predecessor (A) → `[X, A, Y, Z, C, W]`.
- **Yard reorders to `[C, A, B]`**: anchors follow their yard item → `[X, C, W, A, Y, B, Z]`.

## Mutual Exclusivity

`yard:managed`, `yard:overridden` and `yard:omit` are mutually exclusive for a given key. Multiple markers for a given key is treated as a parsing error.

## State outcomes

| Disk state | Yard action |
|---|---|
| managed value equals `default` | in sync; update if desired changed |
| managed value differs from `default` | conflict; block all writes |
| `yard:overridden` | leave untouched |
| `yard:omit ...` | do not emit target |
| desired key/block missing | re-emit |
| desired key exists unmarked | attach marker with yard desired default (may be different to current value); classify on next apply |
| yard no longer wants in-sync item | delete it, omitted keys become "stale keys" |
