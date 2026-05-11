//! `pixi.toml` adaptor — per-key comment marking strategy.
//!
//! Skeleton only at this stage. The semantic surface (channels, platforms,
//! dependencies, tasks, workspace name) lands in Step 2, the reconciler in
//! Step 3.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::RuntimeContext;
use crate::adaptors::ApplyOutcome;

/// Fragment a module wants merged into `pixi.toml`. Empty in Step 1; fields
/// land in Step 2 alongside the merge rules.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct PixiContribution {}

/// Merged intent the adaptor will reconcile against the on-disk `pixi.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct PixiDesired {}

impl PixiDesired {
    pub fn from_contributions<I>(_contribs: I) -> Self
    where
        I: IntoIterator<Item = PixiContribution>,
    {
        Self::default()
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
    ) -> ApplyOutcome {
        // Skeleton: no-op until Step 3 lands the per-key marker logic.
        ApplyOutcome {
            contents: existing.unwrap_or_default().to_string(),
            actions: Vec::new(),
        }
    }
}
