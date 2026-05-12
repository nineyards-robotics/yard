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
use crate::adaptors::pixi::{PixiAdaptor, PixiContribution, PixiDesired};
use crate::{Contribution, ModuleContext, RuntimeContext, YardConfig, modules};

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
/// adaptor's `Desired`, then plan every adaptor's `apply` before committing
/// any writes. The plan-then-commit split is what makes apply atomic — see
/// DESIGN.md ("Apply is atomic ... if any are `Conflict`, the engine
/// surfaces all conflicts ... and writes nothing"). An adaptor with no
/// contributions is currently skipped entirely; once removal lands the
/// adaptor will instead run with an empty `Desired`.
pub fn run(config: &YardConfig, workspace: &Path) -> Result<EngineReport, EngineError> {
    let runtime = RuntimeContext {
        workspace,
        yard_version: env!("CARGO_PKG_VERSION"),
    };
    let module_ctx = ModuleContext {
        config,
        runtime: &runtime,
    };

    let mut gitignore_contribs: Vec<GitignoreContribution> = Vec::new();
    let mut pixi_contribs: Vec<PixiContribution> = Vec::new();

    for module in modules::registry() {
        for contribution in (module.contribute)(&module_ctx) {
            match contribution {
                Contribution::Gitignore(g) => gitignore_contribs.push(g),
                Contribution::Pixi(p) => pixi_contribs.push(p),
            }
        }
    }

    let mut planned: Vec<PlannedFile> = Vec::new();

    if !gitignore_contribs.is_empty() {
        let adaptor = GitignoreAdaptor;
        let path = adaptor.path(&runtime);
        let desired = GitignoreDesired::from_contributions(gitignore_contribs);
        let existing = read_if_exists(&path)?;
        let outcome = adaptor.apply(&desired, existing.as_deref(), &runtime);
        planned.push(PlannedFile { path, outcome });
    }

    if !pixi_contribs.is_empty() {
        let adaptor = PixiAdaptor;
        let path = adaptor.path(&runtime);
        let desired = PixiDesired::from_contributions(pixi_contribs);
        let existing = read_if_exists(&path)?;
        let outcome = adaptor.apply(&desired, existing.as_deref(), &runtime);
        planned.push(PlannedFile { path, outcome });
    }

    let mut report = EngineReport::default();
    for file in planned {
        fs::write(&file.path, &file.outcome.contents).map_err(|source| EngineError::Write {
            path: file.path.clone(),
            source,
        })?;
        report.files.push(FileReport {
            path: file.path,
            actions: file.outcome.actions,
        });
    }

    Ok(report)
}

struct PlannedFile {
    path: PathBuf,
    outcome: crate::adaptors::ApplyOutcome,
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
