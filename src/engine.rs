//! Reconciliation engine: orchestrates a single reconciliation pass.
//!
//! Runs every module, groups contributions per adaptor, merges into each
//! adaptor's `Desired`, applies, and writes results. Both the adaptor and
//! module contracts live with their respective implementations
//! (`crate::adaptors`, `crate::modules`) and the crate-level schema lives in
//! `crate::lib`; the engine depends on all of them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::adaptors::KeyAction;
use crate::adaptors::gitignore::{GitignoreAdaptor, GitignoreContribution, GitignoreDesired};
use crate::{Contribution, YardConfig, modules};

/// Per-file outcome reported back to the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    pub path: PathBuf,
    pub actions: Vec<KeyAction>,
}

/// Aggregate report from one engine run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EngineReport {
    pub files: Vec<FileReport>,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("could not read {path}: {source}", path = .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not write {path}: {source}", path = .path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Run every module, group contributions per adaptor, merge into each
/// adaptor's `Desired`, apply, and write results. An adaptor with no
/// contributions is skipped entirely — its file is not created.
pub fn run(config: &YardConfig, workspace: &Path) -> Result<EngineReport, EngineError> {
    let mut gitignore_contribs: Vec<GitignoreContribution> = Vec::new();

    for module in modules::registry() {
        for contribution in (module.contribute)(config) {
            match contribution {
                Contribution::Gitignore(g) => gitignore_contribs.push(g),
            }
        }
    }

    let mut report = EngineReport::default();

    if !gitignore_contribs.is_empty() {
        let adaptor = GitignoreAdaptor;
        let path = adaptor.path(workspace);
        let desired = GitignoreDesired::from_contributions(gitignore_contribs);
        let existing = read_if_exists(&path)?;
        let outcome = adaptor.apply(&desired, existing.as_deref());
        fs::write(&path, &outcome.contents).map_err(|source| EngineError::Write {
            path: path.clone(),
            source,
        })?;
        report.files.push(FileReport {
            path,
            actions: outcome.actions,
        });
    }

    Ok(report)
}

fn read_if_exists(path: &Path) -> Result<Option<String>, EngineError> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(EngineError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}
