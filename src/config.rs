//! `yard.toml` schema and parsing.
//!
//! `yard.toml` is the user's declaration of intent. It is parsed into
//! [`YardConfig`] once at the start of every yard run; everything downstream
//! (modules, the engine, adaptors) reads from this struct.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Parsed contents of `yard.toml`.
///
/// New top-level keys must always default when absent (see DESIGN.md:
/// "yard never auto-rewrites `yard.toml` to add fields the user didn't ask
/// for"). Unknown keys are rejected via `deny_unknown_fields` so typos
/// surface as errors instead of being silently ignored.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct YardConfig {
    /// ROS 2 distribution to target (e.g. `"jazzy"`). Required, no default.
    pub ros_distro: String,
}

/// Errors that can occur while loading `yard.toml`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read {path}: {source}", path = .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid yard.toml at {path}:\n{source}", path = .path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

impl YardConfig {
    /// Parse a `yard.toml` from an in-memory string.
    pub fn from_str(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Read and parse `yard.toml` from disk, attaching `path` to any error.
    pub fn from_path(path: &Path) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_str(&contents).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }
}
