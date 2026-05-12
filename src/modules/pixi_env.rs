//! `pixi_env` module — emits pixi configuration for the ROS 2 workspace.
//!
//! Derives the pixi environment setup from `yard.toml`:
//! - `workspace.name` from the workspace directory name.
//! - `workspace.channels` from the configured `ros_distro` (conda-forge +
//!   the matching robostack channel).
//! - `dependencies` for the ROS desktop metapackage matching the distro.
//!
//! Pixi is yard's default environment manager — this module is always on
//! and requires no pixi-specific configuration in `yard.toml`.

use std::collections::BTreeMap;

use crate::adaptors::pixi::PixiContribution;
use crate::{Contribution, ModuleContext};

pub fn contribute(ctx: &ModuleContext) -> Vec<Contribution> {
    let distro = ctx.config.ros_distro.as_str();

    let workspace_name = ctx
        .runtime
        .workspace
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());

    let channels = vec![
        "conda-forge".into(),
        format!("robostack-{distro}"),
    ];

    let dependencies = BTreeMap::from([
        ("python".into(), ">=3.11".into()),
        (format!("ros-{distro}-desktop"), "*".into()),
    ]);

    vec![Contribution::Pixi(PixiContribution {
        workspace_name,
        channels,
        dependencies,
    })]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::test_harness::{ModuleHarness, run_module_fixture};

    const HARNESS: ModuleHarness = ModuleHarness {
        fixtures_root: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/modules/pixi_env/fixtures"
        ),
    };

    fn run(name: &str) {
        run_module_fixture(&HARNESS, name, contribute);
    }

    #[test] fn emits_for_jazzy()   { run("emits_for_jazzy"); }
    #[test] fn emits_for_humble()  { run("emits_for_humble"); }
    #[test] fn emits_for_kilted()  { run("emits_for_kilted"); }
    #[test] fn emits_for_rolling() { run("emits_for_rolling"); }
}
