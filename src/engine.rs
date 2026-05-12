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
use crate::{adaptors, modules, Contribution, ModuleContext, RuntimeContext, YardConfig};

/// Per-file outcome reported back to the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    pub path: PathBuf,
    pub outcome: FileOutcome,
    /// Non-blocking notes the adaptor surfaced for this file (e.g. stale
    /// `yard:omit` markers pointing at ids the adaptor no longer emits).
    /// Warnings never block an apply — the engine prints them and moves on.
    pub warnings: Vec<String>,
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
    /// One or more adaptors classified managed content as conflicted. This is
    /// distinct from a planning error: planning succeeded and produced an
    /// action report, but the batch must still be blocked before any commit.
    #[error("{}", format_conflicts(.0))]
    Conflict(Vec<KeyAction>),
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

fn format_conflicts(actions: &[KeyAction]) -> String {
    let mut out = String::new();
    match actions.len() {
        0 => out.push_str("apply blocked by conflict (no detail)"),
        1 => {
            out.push_str("apply blocked by conflict:");
            format_conflict_action(&mut out, &actions[0]);
        }
        n => {
            let _ =
                fmt::Write::write_fmt(&mut out, format_args!("apply blocked by {n} conflicts:"));
            for action in actions {
                format_conflict_action(&mut out, action);
            }
        }
    }
    out
}

fn format_conflict_action(out: &mut String, action: &KeyAction) {
    match action {
        KeyAction::Conflict {
            key,
            on_disk,
            default,
        } => {
            let _ = fmt::Write::write_fmt(
                out,
                format_args!("\n  {key} (on-disk={on_disk:?}, default={default:?})"),
            );
        }
        other => {
            // `EngineError::Conflict` is only constructed from Conflict
            // actions; keep this fallback defensive so future refactors don't
            // panic while formatting an error.
            let _ = fmt::Write::write_fmt(out, format_args!("\n  {other:?}"));
        }
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
        warnings: Vec<String>,
    },
    Delete {
        actions: Vec<KeyAction>,
        warnings: Vec<String>,
    },
    /// Nothing to write or delete. `warnings` is still carried so the
    /// engine can surface them, even when no file change happens.
    Noop { warnings: Vec<String> },
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
            warnings: outcome.warnings,
        },
        (None, true) => PlannedCommit::Delete {
            actions: outcome.actions,
            warnings: outcome.warnings,
        },
        (None, false) => PlannedCommit::Noop {
            warnings: outcome.warnings,
        },
    }
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

fn collect_conflicts(planned: &[PlannedFile]) -> Vec<KeyAction> {
    planned
        .iter()
        .flat_map(|file| match &file.commit {
            PlannedCommit::Write { actions, .. } | PlannedCommit::Delete { actions, .. } => {
                actions.as_slice()
            }
            PlannedCommit::Noop { .. } => &[],
        })
        .filter_map(|action| match action {
            KeyAction::Conflict { .. } => Some(action.clone()),
            _ => None,
        })
        .collect()
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
    run_with_registries(config, workspace, modules::registry(), adaptors::registry())
}

fn run_with_registries(
    config: &YardConfig,
    workspace: &Path,
    module_registry: &[modules::Module],
    adaptor_registry: &[&dyn adaptors::Adaptor],
) -> Result<EngineReport, EngineError> {
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
    for module in module_registry {
        for contribution in (module.contribute)(&module_ctx) {
            by_adaptor
                .entry(contribution.adaptor_id())
                .or_default()
                .push((module.id, contribution));
        }
    }

    let mut planned: Vec<PlannedFile> = Vec::new();
    let mut plan_errors: Vec<PlanError> = Vec::new();

    for adaptor in adaptor_registry {
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

    let conflicts = collect_conflicts(&planned);
    if !conflicts.is_empty() {
        return Err(EngineError::Conflict(conflicts));
    }

    let mut report = EngineReport::default();
    for file in planned {
        match file.commit {
            PlannedCommit::Write {
                contents,
                actions,
                warnings,
            } => {
                fs::write(&file.path, &contents).map_err(|source| EngineError::Write {
                    path: file.path.clone(),
                    source,
                })?;
                report.files.push(FileReport {
                    path: file.path,
                    outcome: FileOutcome::Wrote(actions),
                    warnings,
                });
            }
            PlannedCommit::Delete { actions, warnings } => {
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
                    warnings,
                });
            }
            // No file change to report. Warnings in this branch are
            // structurally unreachable today (an adaptor with no file on
            // disk and no contributions has nothing to warn about), but
            // the assertion catches future drift before warnings get
            // silently dropped.
            PlannedCommit::Noop { warnings } => {
                debug_assert!(
                    warnings.is_empty(),
                    "Noop adaptor outcome produced warnings; extend FileOutcome before allowing this",
                );
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptors::Adaptor;
    use tempfile::TempDir;

    struct WriteAdaptor;
    struct DeleteAdaptor;
    struct ConflictOneAdaptor;
    struct ConflictTwoAdaptor;

    static WRITE_ADAPTOR: WriteAdaptor = WriteAdaptor;
    static DELETE_ADAPTOR: DeleteAdaptor = DeleteAdaptor;
    static CONFLICT_ONE_ADAPTOR: ConflictOneAdaptor = ConflictOneAdaptor;
    static CONFLICT_TWO_ADAPTOR: ConflictTwoAdaptor = ConflictTwoAdaptor;

    impl Adaptor for WriteAdaptor {
        fn id(&self) -> &'static str {
            "write"
        }

        fn path(&self, ctx: &RuntimeContext) -> PathBuf {
            ctx.workspace.join("would-write.txt")
        }

        fn plan(
            &self,
            _contribs: Vec<(&'static str, Contribution)>,
            _existing: Option<&str>,
            _ctx: &RuntimeContext,
        ) -> Result<ApplyOutcome, PlanError> {
            Ok(ApplyOutcome {
                contents: Some("new contents".to_owned()),
                actions: vec![KeyAction::Reemitted {
                    key: "write.key".to_owned(),
                    to: "new contents".to_owned(),
                }],
                warnings: vec![],
            })
        }
    }

    impl Adaptor for DeleteAdaptor {
        fn id(&self) -> &'static str {
            "delete"
        }

        fn path(&self, ctx: &RuntimeContext) -> PathBuf {
            ctx.workspace.join("would-delete.txt")
        }

        fn plan(
            &self,
            _contribs: Vec<(&'static str, Contribution)>,
            _existing: Option<&str>,
            _ctx: &RuntimeContext,
        ) -> Result<ApplyOutcome, PlanError> {
            Ok(ApplyOutcome {
                contents: None,
                actions: vec![KeyAction::Deleted {
                    key: "delete.key".to_owned(),
                    was: "original".to_owned(),
                }],
                warnings: vec![],
            })
        }
    }

    impl Adaptor for ConflictOneAdaptor {
        fn id(&self) -> &'static str {
            "conflict-one"
        }

        fn path(&self, ctx: &RuntimeContext) -> PathBuf {
            ctx.workspace.join("conflict-one.txt")
        }

        fn plan(
            &self,
            _contribs: Vec<(&'static str, Contribution)>,
            _existing: Option<&str>,
            _ctx: &RuntimeContext,
        ) -> Result<ApplyOutcome, PlanError> {
            Ok(ApplyOutcome {
                contents: Some("replacement".to_owned()),
                actions: vec![KeyAction::Conflict {
                    key: "conflict.one".to_owned(),
                    on_disk: "user change".to_owned(),
                    default: "yard default".to_owned(),
                }],
                warnings: vec![],
            })
        }
    }

    impl Adaptor for ConflictTwoAdaptor {
        fn id(&self) -> &'static str {
            "conflict-two"
        }

        fn path(&self, ctx: &RuntimeContext) -> PathBuf {
            ctx.workspace.join("conflict-two.txt")
        }

        fn plan(
            &self,
            _contribs: Vec<(&'static str, Contribution)>,
            _existing: Option<&str>,
            _ctx: &RuntimeContext,
        ) -> Result<ApplyOutcome, PlanError> {
            Ok(ApplyOutcome {
                contents: Some("replacement".to_owned()),
                actions: vec![KeyAction::Conflict {
                    key: "conflict.two".to_owned(),
                    on_disk: "another user change".to_owned(),
                    default: "another yard default".to_owned(),
                }],
                warnings: vec![],
            })
        }
    }

    #[test]
    fn conflicts_block_every_commit_after_planning_all_adaptors() {
        let ws = TempDir::new().unwrap();
        let delete_path = ws.path().join("would-delete.txt");
        fs::write(&delete_path, "original").unwrap();

        let config = YardConfig {
            ros_distro: crate::RosDistro::Jazzy,
        };
        let adaptors: [&dyn Adaptor; 4] = [
            &WRITE_ADAPTOR,
            &DELETE_ADAPTOR,
            &CONFLICT_ONE_ADAPTOR,
            &CONFLICT_TWO_ADAPTOR,
        ];

        let err = run_with_registries(&config, ws.path(), &[], &adaptors).unwrap_err();

        let conflicts = match err {
            EngineError::Conflict(conflicts) => conflicts,
            other => panic!("expected conflict error, got {other:?}"),
        };
        assert_eq!(
            conflicts,
            vec![
                KeyAction::Conflict {
                    key: "conflict.one".to_owned(),
                    on_disk: "user change".to_owned(),
                    default: "yard default".to_owned(),
                },
                KeyAction::Conflict {
                    key: "conflict.two".to_owned(),
                    on_disk: "another user change".to_owned(),
                    default: "another yard default".to_owned(),
                },
            ],
        );

        assert!(
            !ws.path().join("would-write.txt").exists(),
            "planned write should not be committed when any adaptor conflicts",
        );
        assert_eq!(
            fs::read_to_string(delete_path).unwrap(),
            "original",
            "planned delete should not be committed when any adaptor conflicts",
        );
        assert!(
            !ws.path().join("conflict-one.txt").exists(),
            "conflicted adaptor's own planned write should not be committed",
        );
        assert!(
            !ws.path().join("conflict-two.txt").exists(),
            "conflicted adaptor's own planned write should not be committed",
        );
    }
}
