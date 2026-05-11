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
