//! Modules: pure functions from the parsed config to typed contributions.
//!
//! Defines the module contract (`Module`) every concrete module conforms to,
//! plus the registry the engine iterates. Implementations live in submodules
//! below. The crate-level types `YardConfig` and `Contribution` are imported
//! from the crate root — they're yard's vocabulary, not engine-owned.

pub mod pixi_env;
pub mod ros_workspace;

#[cfg(test)]
pub(crate) mod test_harness;

use crate::{Contribution, ModuleContext};

/// A module: an id (used in diagnostics) and a pure function that turns the
/// module context (parsed config + runtime info) into typed contributions.
pub struct Module {
    pub id: &'static str,
    pub contribute: fn(&ModuleContext) -> Vec<Contribution>,
}

/// The ordered set of modules baked into the binary. Iteration order is
/// fixed here — it doesn't change semantics (merges are commutative or
/// error on conflict), but it does fix the order of items in merged
/// `Desired` values for deterministic diffs.
pub fn registry() -> &'static [Module] {
    MODULES
}

static MODULES: &[Module] = &[
    Module {
        id: "ros_workspace",
        contribute: ros_workspace::contribute,
    },
    Module {
        id: "pixi_env",
        contribute: pixi_env::contribute,
    },
];
