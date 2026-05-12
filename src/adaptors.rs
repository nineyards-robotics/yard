//! Adaptors: reconcilers, one per managed output config type.
//!
//! Defines the adaptor contract (`Adaptor`, `ApplyOutcome`, `KeyAction`,
//! `PlanError`, `MergeError`) every concrete adaptor implements.
//! Implementations live in submodules below. Adaptors are independent of
//! the engine — they consume contributions and produce an outcome; the
//! engine just wires them together via [`registry`]. The dependency is
//! one-way: the engine imports from `adaptors`, never the other direction.

use std::path::PathBuf;

use crate::{Contribution, RuntimeContext};

pub mod gitignore;
pub mod pixi;

#[cfg(test)]
pub(crate) mod test_harness;

/// One reconciler. Each adaptor declares a stable `id` (used by
/// [`Contribution::adaptor_id`] to route module contributions) and implements
/// `plan`, which merges its slice of contributions and reconciles them
/// against the on-disk file.
///
/// The engine treats adaptors uniformly: it routes each [`Contribution`] to
/// the adaptor whose id matches, reads the existing file (if any), and calls
/// `plan`. Concrete `Desired`/`ParseError` types stay private to each
/// adaptor — the trait deliberately hides them so the engine and registry
/// don't grow a new variant per adaptor.
pub trait Adaptor: Sync {
    /// Stable identifier, matched against [`Contribution::adaptor_id`].
    fn id(&self) -> &'static str;

    /// Path of the file this adaptor manages, resolved against the runtime
    /// workspace.
    fn path(&self, ctx: &RuntimeContext) -> PathBuf;

    /// Merge `contribs` (each paired with the contributing module's id),
    /// then reconcile against `existing` file contents. `existing` is `None`
    /// when no file is on disk (distinct from `Some("")`, an empty file).
    /// Symmetrically, the returned [`ApplyOutcome::contents`] is `None`
    /// when no file *should* exist: the engine deletes any existing file
    /// or no-ops if there wasn't one. The adaptor — not the engine —
    /// decides whether the managed file exists at all, so the engine stays
    /// agnostic about adaptor-specific create/delete policy.
    ///
    /// Merge failures surface as [`PlanError::Merge`]; parse failures of
    /// the existing file surface as [`PlanError::Parse`] carrying this
    /// adaptor's id.
    fn plan(
        &self,
        contribs: Vec<(&'static str, Contribution)>,
        existing: Option<&str>,
        ctx: &RuntimeContext,
    ) -> Result<ApplyOutcome, PlanError>;
}

/// Ordered set of adaptors baked into the binary. The engine iterates this
/// slice in order; iteration order fixes the order of files in
/// [`EngineReport`](crate::EngineReport) for deterministic output.
pub fn registry() -> &'static [&'static dyn Adaptor] {
    ADAPTORS
}

static ADAPTORS: &[&dyn Adaptor] = &[&gitignore::GitignoreAdaptor, &pixi::PixiAdaptor];

/// Outcome of a single `apply` call.
///
/// `contents` is what the engine will write to disk, or `None` to signal
/// that the managed file should not exist (the engine deletes any existing
/// file or no-ops). `Some("")` is distinct — an empty file the engine will
/// materialise. `actions` records what happened to each managed key/block
/// — used to print human-readable output during `yard apply`, and stays
/// meaningful on deletes (per-fence `Deleted` actions, etc.). `warnings`
/// carries non-blocking notes the user should see — e.g. a `yard:omit`
/// pointing at an id the adaptor no longer emits (DESIGN.md §"Removal":
/// "no file change, reported as Omitted, and yard emits a warning").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub contents: Option<String>,
    pub actions: Vec<KeyAction>,
    pub warnings: Vec<String>,
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
    /// Key/block carries a `yard:overridden` marker: the user explicitly took
    /// ownership and yard never touches it. Carries no `default=` payload —
    /// see DESIGN.md ("the marker carries no default= payload").
    Overridden { key: String },
    /// Key/block was previously managed by yard but the adaptor no longer
    /// wants it — yard removed it. `was` records what got deleted so the
    /// CLI can narrate the change.
    Deleted { key: String, was: String },
    /// User wrote `# yard:omit <key>`; yard skipped this id (either
    /// because it would otherwise re-emit, or because it has nothing to
    /// emit and the omit is stale — the latter also produces a warning).
    Omitted { key: String },
}

/// One planning-phase failure surfaced by [`Adaptor::plan`]. The engine
/// collects these across every adaptor before deciding to commit, so the
/// user sees all problems at once.
///
/// `Parse` is type-erased so the engine and its `EngineError::Plan(..)`
/// envelope don't grow a variant per adaptor — the `adaptor` field tags
/// which adaptor produced the source error. `Merge` carries the structured
/// [`MergeError`] every adaptor's `from_contributions` returns; it is
/// reachable as soon as an adaptor surfaces scalar-merge conflicts (v1's
/// gitignore is additive only, so `Merge` is currently unreachable in
/// practice).
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("{source}")]
    Parse {
        adaptor: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error(transparent)]
    Merge(#[from] MergeError),
}

/// Two modules disagreed on a scalar contributed to the same adaptor (see
/// DESIGN.md: "Scalars ... error if two modules disagree, naming both
/// modules and the offending key. Conflicts are loud, not silent.").
///
/// Returned by each adaptor's `from_contributions`. Unreachable in v1's
/// gitignore (additive merge only); the type lives now so pixi can use it
/// without a contract refactor when its scalar fields land.
#[derive(Debug, thiserror::Error)]
#[error(
    "merge conflict on `{key}` in {adaptor}: modules {modules:?} disagree (values {values:?})"
)]
pub struct MergeError {
    pub adaptor: &'static str,
    pub key: String,
    pub modules: Vec<&'static str>,
    pub values: Vec<String>,
}
