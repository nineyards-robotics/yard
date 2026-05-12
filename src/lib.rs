//! yard — ROS 2 workspace orchestrator.
//!
//! Three top-level modules:
//! - `engine` — the reconciliation loop
//! - `adaptors` — concrete reconcilers, one per managed output file type
//! - `modules` — concrete opinion-emitters
//!
//! Four crate-level types are yard's vocabulary and live here at the root:
//! `YardConfig` (the parsed `yard.toml`), `Contribution` (the typed handshake
//! from modules to adaptors), and the two context types — `RuntimeContext`
//! (what's true about this invocation) and `ModuleContext` (user config plus
//! runtime context, what a module sees). Adaptors are given `RuntimeContext`
//! only — never `YardConfig`. The whole point of the module → contribution →
//! adaptor pipeline is that adaptors consume typed intent through `Desired`,
//! never the raw user config.
//!
//! `src/main.rs` is a thin CLI shell over this library.

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub mod adaptors;
pub mod engine;
pub mod modules;

#[cfg(test)]
mod test_support;

pub use engine::{EngineError, EngineReport, FileOutcome, FileReport};

/// Supported ROS 2 distributions yard can target.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RosDistro {
    Humble,
    Jazzy,
    Kilted,
    Rolling,
}

impl RosDistro {
    /// Lowercase string form matching serde's `rename_all = "lowercase"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Humble => "humble",
            Self::Jazzy => "jazzy",
            Self::Kilted => "kilted",
            Self::Rolling => "rolling",
        }
    }
}

/// Parsed contents of `yard.toml`.
///
/// New top-level keys must always default when absent (see DESIGN.md:
/// "yard never auto-rewrites `yard.toml` to add fields the user didn't ask
/// for"). Unknown keys are rejected via `deny_unknown_fields` so typos
/// surface as errors instead of being silently ignored.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct YardConfig {
    /// ROS 2 distribution to target. Required, no default.
    pub ros_distro: RosDistro,
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

impl FromStr for YardConfig {
    type Err = toml::de::Error;

    /// Parse a `yard.toml` from an in-memory string.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        toml::from_str(s)
    }
}

impl YardConfig {
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

/// Typed contribution fragments emitted by modules.
///
/// Each variant targets exactly one adaptor; [`Contribution::adaptor_id`] is
/// the routing key the engine uses to fan contributions out to
/// `adaptors::registry()`. Adding a new adaptor means: add a variant, add a
/// match arm to `adaptor_id`, and register the adaptor — no engine changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Contribution {
    Gitignore(crate::adaptors::gitignore::GitignoreContribution),
    Pixi(crate::adaptors::pixi::PixiContribution),
}

impl Contribution {
    /// Id of the adaptor this contribution targets. Must match the
    /// corresponding [`Adaptor::id`](crate::adaptors::Adaptor::id).
    pub fn adaptor_id(&self) -> &'static str {
        match self {
            Contribution::Gitignore(_) => crate::adaptors::gitignore::GitignoreAdaptor::ID,
            Contribution::Pixi(_) => crate::adaptors::pixi::PixiAdaptor::ID,
        }
    }
}

/// Runtime context for one yard invocation: everything that's true about the
/// run other than the user's parsed config. Both modules and adaptors see
/// this. Kept deliberately lean — only fields with a known consumer land
/// here.
pub struct RuntimeContext<'a> {
    /// Workspace directory (where `yard.toml` lives).
    pub workspace: &'a Path,
    /// Version of the running yard binary (`CARGO_PKG_VERSION`).
    pub yard_version: &'static str,
}

/// What a module sees during `contribute`: the parsed user config plus the
/// runtime context. Adaptors deliberately do *not* receive `config` — they
/// consume typed `Desired` values, so the boundary between user intent and
/// file-format concerns stays honest.
pub struct ModuleContext<'a> {
    pub config: &'a YardConfig,
    pub runtime: &'a RuntimeContext<'a>,
}
