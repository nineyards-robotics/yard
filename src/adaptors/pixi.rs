//! `pixi.toml` adaptor — per-key comment marking strategy.
//!
//! Skeleton only at this stage. The semantic surface (channels, platforms,
//! dependencies, tasks, workspace name) lands in Step 2, the reconciler in
//! Step 3.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::RuntimeContext;
use crate::adaptors::ApplyOutcome;
use crate::engine::MergeError;

/// Fragment a module wants merged into `pixi.toml`. Empty in Step 1; fields
/// land in Step 2 alongside the merge rules.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct PixiContribution {}

/// Merged intent the adaptor will reconcile against the on-disk `pixi.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct PixiDesired {}

/// Placeholder parse error for `pixi.toml`. Empty until the reconciler lands
/// in Step 3 — kept for shape symmetry with `GitignoreParseError` so the
/// engine's `PlanError::Parse` envelope doesn't need to grow new variants
/// the day the first real pixi parse failure appears.
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
            contents: existing.unwrap_or_default().to_string(),
            actions: Vec::new(),
        })
    }
}
