//! Reconciliation engine: orchestrates a single reconciliation pass.
//!
//! Runs every module, routes each contribution to the adaptor named by
//! [`Contribution::adaptor_id`], then plans every adaptor in
//! `adaptors::registry()` before committing any writes. The engine itself
//! knows nothing about specific adaptors — adding one is a registry entry,
//! not an engine change. Both the adaptor and module contracts live with
//! their respective implementations (`crate::adaptors`, `crate::modules`)
//! and the crate-level schema lives in `crate::lib`.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::adaptors::{ApplyOutcome, KeyAction, PlanError};
use crate::{Contribution, ModuleContext, RuntimeContext, YardConfig, adaptors, modules};

/// Per-file outcome reported back to the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    pub path: PathBuf,
    pub outcome: FileOutcome,
}

/// What happened to one managed file during the commit phase. Both arms
/// carry the per-key actions the adaptor reported — `Deleted` keeps them
/// so the CLI can still narrate which managed keys/blocks went away when
/// per-fence removal lands. Files that didn't exist before and still
/// shouldn't are silently skipped and never appear in the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOutcome {
    Wrote(Vec<KeyAction>),
    Deleted(Vec<KeyAction>),
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
    #[error("could not delete {path}: {source}", path = .path.display())]
    Delete {
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

/// Run every module, route each contribution to its adaptor by id, then
/// plan every adaptor in registry order before committing any writes. The
/// plan-then-commit split is what makes apply atomic — see DESIGN.md
/// ("Apply is atomic ... if any are `Conflict`, the engine surfaces all
/// conflicts ... and writes nothing"). Every adaptor runs on every apply,
/// including with no contributions — the adaptor decides via
/// [`ApplyOutcome::contents`] whether the file should exist. `Some(_)`
/// means "write this content"; `None` means "no file should exist", and
/// the engine deletes any existing file or no-ops if there wasn't one
/// (DESIGN.md §"Removal").
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
    // modules and the offending key"). Routing key is the adaptor id, so
    // adding a new adaptor doesn't require touching this loop.
    let mut by_adaptor: HashMap<&'static str, Vec<(&'static str, Contribution)>> = HashMap::new();
    for module in modules::registry() {
        for contribution in (module.contribute)(&module_ctx) {
            by_adaptor
                .entry(contribution.adaptor_id())
                .or_default()
                .push((module.id, contribution));
        }
    }

    let mut planned: Vec<PlannedFile> = Vec::new();
    let mut plan_errors: Vec<PlanError> = Vec::new();

    for adaptor in adaptors::registry() {
        let contribs = by_adaptor.remove(adaptor.id()).unwrap_or_default();
        let path = adaptor.path(&runtime);
        let existing = read_if_exists(&path)?;
        let existed = existing.is_some();
        match adaptor.plan(contribs, existing.as_deref(), &runtime) {
            Ok(outcome) => planned.push(PlannedFile {
                path,
                commit: resolve_commit(outcome, existed),
            }),
            Err(e) => plan_errors.push(e),
        }
    }

    if !plan_errors.is_empty() {
        return Err(EngineError::Plan(plan_errors));
    }

    let mut report = EngineReport::default();
    for file in planned {
        match file.commit {
            PlannedCommit::Write { contents, actions } => {
                fs::write(&file.path, &contents).map_err(|source| EngineError::Write {
                    path: file.path.clone(),
                    source,
                })?;
                report.files.push(FileReport {
                    path: file.path,
                    outcome: FileOutcome::Wrote(actions),
                });
            }
            PlannedCommit::Delete { actions } => {
                match fs::remove_file(&file.path) {
                    Ok(()) => {}
                    // Raced with someone else deleting it — treat the
                    // observed end state as success.
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(source) => {
                        return Err(EngineError::Delete {
                            path: file.path,
                            source,
                        });
                    }
                }
                report.files.push(FileReport {
                    path: file.path,
                    outcome: FileOutcome::Deleted(actions),
                });
            }
            PlannedCommit::Noop => {}
        }
    }

    Ok(report)
}

/// Collapse the adaptor's typed intent (`ApplyOutcome`) plus plan-time
/// knowledge of file existence into a single typed commit action. Doing
/// this at plan time keeps the commit loop a clean three-way match — no
/// loose `existed` flag to carry past planning.
fn resolve_commit(outcome: ApplyOutcome, existed: bool) -> PlannedCommit {
    match (outcome.contents, existed) {
        (Some(contents), _) => PlannedCommit::Write {
            contents,
            actions: outcome.actions,
        },
        (None, true) => PlannedCommit::Delete {
            actions: outcome.actions,
        },
        (None, false) => PlannedCommit::Noop,
    }
}

struct PlannedFile {
    path: PathBuf,
    commit: PlannedCommit,
}

/// What the commit phase will do to one managed file. Resolved once at
/// plan time by [`resolve_commit`] so the commit loop doesn't have to
/// re-derive "delete vs no-op" from the adaptor's `Option<String>`
/// contents plus a separate existence flag.
enum PlannedCommit {
    Write {
        contents: String,
        actions: Vec<KeyAction>,
    },
    Delete {
        actions: Vec<KeyAction>,
    },
    Noop,
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
