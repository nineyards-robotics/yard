//! Adaptors: reconcilers, one per managed output config type.
//!
//! Defines the adaptor contract (`ApplyOutcome`, `KeyAction`) every concrete
//! adaptor returns from `apply`. Implementations live in submodules below.
//! Adaptors are independent of the engine — they consume a typed `Desired`
//! and produce an outcome; the engine wires them together.

pub mod gitignore;

#[cfg(test)]
pub(crate) mod test_harness;

/// Outcome of a single `apply` call.
///
/// `contents` is what the engine will write to disk. `actions` records what
/// happened to each managed key/block — used to print human-readable output
/// during `yard apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub contents: String,
    pub actions: Vec<KeyAction>,
}

/// Per-key (or per-block) action taken by an adaptor.
///
/// Values are stringly-typed for now. Once a structured-config adaptor lands
/// (pixi.toml, pre-commit-config.yaml) this will likely become an associated
/// type or carry richer payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Key/block already matches what yard would emit; no rewrite needed.
    InSync { key: String },
    /// Key/block existed but differed; yard rewrote it.
    Updated {
        key: String,
        from: String,
        to: String,
    },
    /// Key/block was absent (or the file did not exist); yard emitted it.
    Reemitted { key: String, to: String },
    /// Key/block carries a `yard:frozen` marker; yard left it untouched.
    Frozen { key: String },
}
