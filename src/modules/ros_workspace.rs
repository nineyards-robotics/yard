//! `ros_workspace` module — always-on, emits the standard ROS 2
//! build-artifact ignores.
//!
//! Every ROS 2 workspace built with `colcon` produces `build/`, `install/`,
//! and `log/`. Ignoring them is universal enough that this module emits them
//! unconditionally; nothing in `yard.toml` toggles it.

use crate::adaptors::gitignore::GitignoreContribution;
use crate::{Contribution, YardConfig};

pub fn contribute(_config: &YardConfig) -> Vec<Contribution> {
    vec![Contribution::Gitignore(GitignoreContribution {
        lines: vec!["build/".into(), "install/".into(), "log/".into()],
    })]
}

#[cfg(test)]
mod tests {
    //! Add a new scenario: drop a directory under `fixtures/` containing
    //! `yard.toml`, then add a one-line `#[test]` below.
    //! `expected.contributions.ron` is generated on first run with
    //! `UPDATE_GOLDENS=1`.

    use super::*;
    use crate::modules::test_harness::{ModuleHarness, run_module_fixture};

    const HARNESS: ModuleHarness = ModuleHarness {
        fixtures_root: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/modules/ros_workspace/fixtures"
        ),
    };

    fn run(name: &str) {
        run_module_fixture(&HARNESS, name, contribute);
    }

    #[test] fn emits_standard_ignores() { run("emits_standard_ignores"); }
}
