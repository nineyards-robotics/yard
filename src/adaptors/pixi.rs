//! `pixi.toml` adaptor — per-key comment marking strategy.
//!
//! Skeleton only at this stage. The semantic surface (channels, platforms,
//! dependencies, tasks, workspace name) lands in Step 2, the reconciler in
//! Step 3.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::adaptors::{Adaptor, ApplyOutcome, MergeError, PlanError};
use crate::{Contribution, RuntimeContext};

/// Fragment a module wants merged into `pixi.toml`. Empty in Step 1; fields
/// land in Step 2 alongside the merge rules.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct PixiContribution {}

/// Merged intent the adaptor will reconcile against the on-disk `pixi.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct PixiDesired {}

/// Placeholder parse error for `pixi.toml`. Empty until the reconciler lands
/// in Step 3 — kept for shape symmetry with `GitignoreParseError` so the
/// type-erased `PlanError::Parse` envelope doesn't need any changes the day
/// the first real pixi parse failure appears.
#[derive(Debug, thiserror::Error)]
#[error("pixi parse error (unreachable until Step 3 lands)")]
pub struct PixiParseError;

impl PixiDesired {
    pub fn from_contributions<I>(_contribs: I) -> Result<Self, MergeError>
    where
        I: IntoIterator<Item = (&'static str, PixiContribution)>,
    {
        Ok(Self::default())
    }
}

pub struct PixiAdaptor;

impl PixiAdaptor {
    pub const ID: &'static str = "pixi";

    pub fn path(&self, ctx: &RuntimeContext) -> PathBuf {
        ctx.workspace.join("pixi.toml")
    }

    pub fn apply(
        &self,
        _desired: &PixiDesired,
        existing: Option<&str>,
        _ctx: &RuntimeContext,
    ) -> Result<ApplyOutcome, PixiParseError> {
        // Skeleton: no-op until Step 3 lands the per-key marker logic.
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
        // Skeleton: with no file on disk, signal "no file" so the engine
        // doesn't create an empty `pixi.toml`. Real logic — including
        // when to *actively* delete an existing pixi.toml — lands in
        // Step 3.
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
