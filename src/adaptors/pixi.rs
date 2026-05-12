//! `pixi.toml` adaptor — per-key comment marking strategy.
//!
//! yard owns individual keys inside `pixi.toml`, each carrying a trailing
//! `# yard:managed default=<...>` marker. The user owns the surrounding
//! table headers, blank lines, comments, and any keys yard does not manage.
//!
//! v1 surface (per DESIGN.md):
//! - **Scalar**: `workspace.name` → `name = "<v>"  # yard:managed default="<v>"`.
//! - **Array**: `workspace.channels` → element-level reconciliation. User-added
//!   elements survive; removing one of yard's elements is a `Conflict`.
//! - **Map of scalars**: each entry under `[dependencies]` is its own managed
//!   key (`dependencies.<name>`); the `[dependencies]` *table* itself is the
//!   user's, so untracked deps are preserved verbatim.
//!
//! Step 1 (this commit) defines the typed surface plus an exhaustive test
//! suite; Step 2 lands the reconciler that turns those tests green.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::adaptors::{Adaptor, ApplyOutcome, MergeError, PlanError};
use crate::{Contribution, RuntimeContext};

/// Fragment a module wants merged into `pixi.toml`.
///
/// Every field is optional/empty by default so a module only mentions what
/// it actually wants. Merging is per-field: scalars must agree across
/// modules (else `MergeError`), arrays union with dedup, maps merge with
/// per-key agreement.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PixiContribution {
    /// Scalar at `[workspace] name`.
    pub workspace_name: Option<String>,
    /// Array at `[workspace] channels`. Element-level reconciliation —
    /// user-added entries survive across applies.
    pub channels: Vec<String>,
    /// Map at `[dependencies]`. Keys are dep names, values are version
    /// constraints (e.g. `"3.11"`, `">=1.20"`, `"*"`).
    pub dependencies: BTreeMap<String, String>,
}

/// Merged intent the adaptor reconciles against the on-disk `pixi.toml`.
///
/// Shape mirrors [`PixiContribution`]; the merge collapses an iterator of
/// contributions into one of these. `from_contributions` enforces the
/// scalar-agreement rule per DESIGN.md §Modules→Merge.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PixiDesired {
    pub workspace_name: Option<String>,
    pub channels: Vec<String>,
    pub dependencies: BTreeMap<String, String>,
}

/// Parse error surfaced when reconciling against a malformed `pixi.toml`.
///
/// Per DESIGN.md §Classification, marker malformations are loud parse errors
/// rather than silent re-classification — the file fails loud rather than
/// letting yard silently take or lose ownership. The two malformations that
/// surface here are a `yard:managed` with no `default=` payload at all and a
/// `default=` whose serialized form parses but to the wrong shape for the
/// key it annotates.
///
/// A `default=` that fails to parse as TOML *at all* deliberately does not
/// surface here: yard treats it as if the marker were missing, falling into
/// the "unmarked" classification row and re-emitting a fresh marker with
/// `default=<desired>`. Self-healing across yard upgrades wins over loud
/// failure, and the on-disk value still gets re-asserted on the next apply
/// (any value drift then surfaces as `Conflict`).
#[derive(Debug, thiserror::Error)]
#[error("{path}: {kind}", path = .path.display())]
pub struct PixiParseError {
    pub path: PathBuf,
    pub kind: PixiParseErrorKind,
}

#[derive(Debug, thiserror::Error)]
pub enum PixiParseErrorKind {
    #[error("invalid TOML: {0}")]
    InvalidToml(String),
    #[error("`{key}` carries `yard:managed` without a `default=` payload")]
    ManagedMissingDefault { key: String },
    #[error(
        "`{key}`'s `default=` payload is a {default_shape} but the key holds a {value_shape}"
    )]
    DefaultShapeMismatch {
        key: String,
        default_shape: &'static str,
        value_shape: &'static str,
    },
}

impl PixiDesired {
    /// Collapse per-module contributions into a single merged desired.
    ///
    /// Rules per DESIGN.md §Modules→Merge:
    /// - **Scalars** (`workspace_name`): all contributors must agree.
    /// - **Arrays** (`channels`): union with order preserved; later
    ///   modules' net-new items append, dedup is on equality.
    /// - **Maps** (`dependencies`): per-key union; same key from two
    ///   modules must agree on value.
    pub fn from_contributions<I>(_contribs: I) -> Result<Self, MergeError>
    where
        I: IntoIterator<Item = (&'static str, PixiContribution)>,
    {
        // Step 2: real merge. Today this throws away inputs so the merge
        // tests stay red.
        Ok(Self::default())
    }
}

pub struct PixiAdaptor;

impl PixiAdaptor {
    pub const ID: &'static str = "pixi";

    pub fn path(&self, ctx: &RuntimeContext) -> PathBuf {
        ctx.workspace.join("pixi.toml")
    }

    /// Apply the typed desired against the on-disk file. Step 1 stub: every
    /// fixture's `expected.pixi.toml` records what the *real* reconciler
    /// should emit, so this no-op makes every fixture red. Step 2 lands the
    /// real reconciler.
    pub fn apply(
        &self,
        _desired: &PixiDesired,
        existing: Option<&str>,
        _ctx: &RuntimeContext,
    ) -> Result<ApplyOutcome, PixiParseError> {
        Ok(ApplyOutcome {
            contents: Some(existing.unwrap_or_default().to_string()),
            actions: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

impl Adaptor for PixiAdaptor {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn path(&self, ctx: &RuntimeContext) -> PathBuf {
        PixiAdaptor::path(self, ctx)
    }

    fn plan(
        &self,
        contribs: Vec<(&'static str, Contribution)>,
        existing: Option<&str>,
        ctx: &RuntimeContext,
    ) -> Result<ApplyOutcome, PlanError> {
        let mine = contribs.into_iter().map(|(module, c)| match c {
            Contribution::Pixi(p) => (module, p),
            other => unreachable!(
                "engine routed {} contribution to pixi adaptor",
                other.adaptor_id()
            ),
        });
        let desired = PixiDesired::from_contributions(mine)?;
        // Step 1 stub: with no file on disk and no managed keys, signal
        // "no file" so the engine doesn't create an empty `pixi.toml`.
        // Step 2 makes this decision based on the merged desired.
        if existing.is_none() {
            return Ok(ApplyOutcome {
                contents: None,
                actions: Vec::new(),
                warnings: Vec::new(),
            });
        }
        self.apply(&desired, existing, ctx).map_err(|e| PlanError::Parse {
            adaptor: Self::ID,
            source: Box::new(e),
        })
    }
}

#[cfg(test)]
mod tests;
