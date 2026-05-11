//! yard — ROS 2 workspace orchestrator.
//!
//! All real logic lives here as a library; `src/main.rs` is a thin CLI shell.

pub mod config;

pub use config::{ConfigError, YardConfig};
