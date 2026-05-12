//! Reconciliation engine: orchestrates a single reconciliation pass.
//!
//! Runs every module, groups contributions per adaptor, merges into each
//! adaptor's `Desired`, applies, and writes results. Both the adaptor and
//! module contracts live with their respective implementations
//! (`crate::adaptors`, `crate::modules`) and the crate-level schema lives in
//! `crate::lib`; the engine depends on all of them.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::adaptors::KeyAction;
use crate::adaptors::gitignore::{
    GitignoreAdaptor, GitignoreContribution, GitignoreDesired, GitignoreParseError,
};
use crate::adaptors::pixi::{PixiAdaptor, PixiContribution, PixiDesired, PixiParseError};
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
    /// Planning phase produced one or more errors. The engine collects errors
    /// from every adaptor before deciding to write, so the user sees all
    /// problems at once. Per DESIGN.md ("Apply is atomic ... writes nothing"),
    /// any non-empty vec here blocks every write across every adaptor.
    #[error("{}", format_plan_errors(.0))]
    Plan(Vec<PlanError>),
}

/// One planning-phase failure. Future variants (e.g. pixi's `Conflict` —
/// `actual != default=` per DESIGN.md's central rule) sit alongside `Parse`
/// and `Merge` rather than being nested under one of them.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Merge(#[from] MergeError),
}

/// Adaptor-specific parse failure (file content is malformed and the adaptor
/// couldn't even classify it). Each adaptor's parse error type stays its own
/// — they share the *class* "parse" but not a payload, so file-format
/// concerns don't leak across adaptor boundaries.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error(transparent)]
    Gitignore(#[from] GitignoreParseError),
    #[error(transparent)]
    Pixi(#[from] PixiParseError),
}

/// Two modules disagreed on a scalar contributed to the same adaptor (see
/// DESIGN.md: "Scalars ... error if two modules disagree, naming both
/// modules and the offending key. Conflicts are loud, not silent.").
///
/// Unreachable in v1's gitignore (additive merge only); the type lives now
/// so pixi can use it without an engine refactor when its scalar fields
/// land.
#[derive(Debug, thiserror::Error)]
#[error(
    "merge conflict on `{key}` in {adaptor}: modules {modules:?} disagree (values {values:?})"
)]
pub struct MergeError {
    pub adaptor: &'static str,
    pub key: String,
    pub modules: Vec<&'static str>,
    pub values: Vec<String>,
}

fn format_plan_errors(errors: &[PlanError]) -> String {
    let mut out = String::new();
    match errors.len() {
        0 => out.push_str("plan failed (no detail)"),
        1 => {
            // Single-error case reads cleanly without a count prefix.
            let _ = fmt::Write::write_fmt(&mut out, format_args!("{}", errors[0]));
        }
        n => {
            let _ = fmt::Write::write_fmt(&mut out, format_args!("{n} planning errors:"));
            for e in errors {
                let _ = fmt::Write::write_fmt(&mut out, format_args!("\n  {e}"));
            }
        }
    }
    out
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

    // Contributions are paired with their source module id so any merge
    // conflict downstream can name both modules (DESIGN.md "naming both
    // modules and the offending key").
    let mut gitignore_contribs: Vec<(&'static str, GitignoreContribution)> = Vec::new();
    let mut pixi_contribs: Vec<(&'static str, PixiContribution)> = Vec::new();

    for module in modules::registry() {
        for contribution in (module.contribute)(&module_ctx) {
            match contribution {
                Contribution::Gitignore(g) => gitignore_contribs.push((module.id, g)),
                Contribution::Pixi(p) => pixi_contribs.push((module.id, p)),
            }
        }
    }

    let mut planned: Vec<PlannedFile> = Vec::new();
    let mut plan_errors: Vec<PlanError> = Vec::new();

    if !gitignore_contribs.is_empty() {
        plan_one(
            GitignoreAdaptor.path(&runtime),
            GitignoreDesired::from_contributions(gitignore_contribs),
            |desired, existing| GitignoreAdaptor.apply(desired, existing, &runtime),
            &mut planned,
            &mut plan_errors,
        )?;
    }

    if !pixi_contribs.is_empty() {
        plan_one(
            PixiAdaptor.path(&runtime),
            PixiDesired::from_contributions(pixi_contribs),
            |desired, existing| PixiAdaptor.apply(desired, existing, &runtime),
            &mut planned,
            &mut plan_errors,
        )?;
    }

    if !plan_errors.is_empty() {
        return Err(EngineError::Plan(plan_errors));
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

/// Run one adaptor's planning step: merge contributions, read existing file
/// (if any), run `apply`. Merge and parse errors are pushed onto
/// `plan_errors` rather than returned, so the engine can collect failures
/// across every adaptor before deciding to write. Read failures still
/// short-circuit via `?` — they're IO problems, not planning errors, and
/// fail-fast keeps the error envelope honest.
fn plan_one<D, E, F>(
    path: PathBuf,
    merged: Result<D, MergeError>,
    apply: F,
    planned: &mut Vec<PlannedFile>,
    plan_errors: &mut Vec<PlanError>,
) -> Result<(), EngineError>
where
    F: FnOnce(&D, Option<&str>) -> Result<crate::adaptors::ApplyOutcome, E>,
    PlanError: From<E>,
{
    let desired = match merged {
        Ok(d) => d,
        Err(e) => {
            // MergeError is concrete; the From<E> bound covers the apply
            // side only, so we construct the variant directly here.
            plan_errors.push(PlanError::Merge(e));
            return Ok(());
        }
    };
    let existing = read_if_exists(&path)?;
    match apply(&desired, existing.as_deref()) {
        Ok(outcome) => planned.push(PlannedFile { path, outcome }),
        Err(e) => plan_errors.push(PlanError::from(e)),
    }
    Ok(())
}

// Adaptor parse errors fan in to PlanError through ParseError.
impl From<GitignoreParseError> for PlanError {
    fn from(e: GitignoreParseError) -> Self {
        PlanError::Parse(ParseError::Gitignore(e))
    }
}

impl From<PixiParseError> for PlanError {
    fn from(e: PixiParseError) -> Self {
        PlanError::Parse(ParseError::Pixi(e))
    }
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
