//! `.gitignore` adaptor — block-fencing marking strategy.
//!
//! yard owns the lines inside each `# >>> yard:managed id=<slug> >>>` /
//! `# <<< yard:managed id=<slug> <<<` block. Everything outside is the
//! user's and is preserved verbatim. A fence whose markers are rewritten
//! as `yard:overridden id=<slug>` is the user explicitly taking the block
//! over — yard never touches its interior.
//!
//! Every fence carries a required `id=<slug>` (`[A-Za-z0-9_-]+`). The id
//! identifies what the block contains so multiple managed blocks can
//! coexist in one file. yard's territory is one fence per id; the user owns
//! anything outside any fence.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::adaptors::{Adaptor, ApplyOutcome, KeyAction, MergeError, PlanError};
use crate::{Contribution, RuntimeContext};

/// Fragment a module wants merged into the gitignore. Targets one fence —
/// `id` names the block (`[A-Za-z0-9_-]+`); `lines` are raw gitignore
/// patterns (e.g. `"build/"`) yard will own inside that fence.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct GitignoreContribution {
    pub id: String,
    pub lines: Vec<String>,
}

/// One managed fence the adaptor will write: an id plus the lines inside.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DesiredFence {
    pub id: String,
    pub lines: Vec<String>,
}

/// Merged intent the adaptor reconciles against the on-disk gitignore.
///
/// Fences are grouped by id (first contribution with a given id fixes the
/// fence's position; later contributions with the same id append their
/// lines, dropping duplicates). Different ids stay separate.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct GitignoreDesired {
    pub fences: Vec<DesiredFence>,
}

impl GitignoreDesired {
    /// Merge per-module contributions into a single desired set of fences.
    ///
    /// Each input is paired with its contributing module id so any future
    /// scalar-merge conflict can name both parties (see DESIGN.md: "Scalars
    /// ... error if two modules disagree, naming both modules and the
    /// offending key"). gitignore's merge is purely additive — lines for a
    /// given fence id union with dedup — so this always returns `Ok`; the
    /// `Result` is here for shape symmetry with adaptors that do have
    /// scalars.
    pub fn from_contributions<I>(contribs: I) -> Result<Self, MergeError>
    where
        I: IntoIterator<Item = (&'static str, GitignoreContribution)>,
    {
        let mut fences: Vec<DesiredFence> = Vec::new();
        for (_module, c) in contribs {
            if let Some(existing) = fences.iter_mut().find(|f| f.id == c.id) {
                for line in c.lines {
                    if !existing.lines.iter().any(|l| l == &line) {
                        existing.lines.push(line);
                    }
                }
            } else {
                fences.push(DesiredFence {
                    id: c.id,
                    lines: c.lines,
                });
            }
        }
        Ok(Self { fences })
    }
}

pub struct GitignoreAdaptor;

impl GitignoreAdaptor {
    pub const ID: &'static str = "gitignore";

    pub fn path(&self, ctx: &RuntimeContext) -> PathBuf {
        ctx.workspace.join(".gitignore")
    }

    pub fn apply(
        &self,
        desired: &GitignoreDesired,
        existing: Option<&str>,
        ctx: &RuntimeContext,
    ) -> Result<ApplyOutcome, GitignoreParseError> {
        let mut contents = existing.unwrap_or("").to_string();
        let mut actions = Vec::new();
        let mut warnings = Vec::new();
        let path = self.path(ctx);

        let parse = |c: &str| {
            parse_markers(c).map_err(|p| GitignoreParseError {
                path: path.clone(),
                line: p.line,
                kind: p.kind,
            })
        };

        // Omits are a property of the *file* (line comments), not of any
        // particular fence — capture which ids the user wants suppressed
        // up front. Malformed omits (`# yard:omit foo.bar`) become
        // warnings only; they never block emission.
        let initial = parse(&contents)?;
        let mut omit_ids: Vec<String> = Vec::new();
        for omit in &initial.omits {
            match &omit.target {
                OmitTarget::Slug(id) => omit_ids.push(id.clone()),
                OmitTarget::Invalid(raw) => warnings.push(format!(
                    "yard:omit `{raw}` is not a valid id slug (allowed: [A-Za-z0-9_-]+); ignoring"
                )),
            }
        }

        let desired_ids: Vec<&str> = desired.fences.iter().map(|f| f.id.as_str()).collect();

        // Desired walk — reconcile each desired fence against the on-disk
        // file. Skip ids in `omit_ids`: the user said no, so we do not
        // emit. Any on-disk managed fence with an omitted id is removed
        // in the cleanup pass below.
        for fence in &desired.fences {
            if omit_ids.iter().any(|o| o == &fence.id) {
                continue;
            }
            // Re-parse for each fence: a previous splice/append may have
            // shifted byte offsets. Cheap for the file sizes we deal
            // with and keeps the splice math correct without offset
            // bookkeeping. Splices only touch fence interiors, so a
            // file that parsed once parses the same way on every
            // iteration.
            let parsed = parse(&contents)?;
            let inner = render_inner(&fence.lines);
            match parsed.find_fence(&fence.id) {
                Some(found) if found.kind == FenceKind::Overridden => {
                    actions.push(KeyAction::Overridden {
                        key: fence.id.clone(),
                    });
                }
                Some(found) if found.inner == inner => {
                    actions.push(KeyAction::InSync {
                        key: fence.id.clone(),
                    });
                }
                Some(found) => {
                    let from = found.inner.clone();
                    let new_block = render_block(&fence.id, &inner);
                    contents = splice(&contents, found.open_start, found.close_end, &new_block);
                    actions.push(KeyAction::Updated {
                        key: fence.id.clone(),
                        from,
                        to: inner,
                    });
                }
                None => {
                    contents = append_block(&contents, &fence.id, &inner);
                    actions.push(KeyAction::Reemitted {
                        key: fence.id.clone(),
                        to: inner,
                    });
                }
            }
        }

        // Cleanup pass: walk every on-disk fence the desired-walk did not
        // touch. This catches three cases:
        //  - stale managed (id not in desired, not omitted) → delete.
        //  - stale overridden (id not in desired) → preserved, reported.
        //  - omitted managed (id in omits, regardless of desired) →
        //    delete and warn that the omit took effect. Overridden
        //    fences with an omit are left alone — yard never touches
        //    overridden content.
        // Actions are pushed in on-disk (top-down) order so the report
        // mirrors what the user sees in the file. Excise is done
        // bottom-up afterwards so byte offsets stay valid.
        let cleanup = parse(&contents)?;
        let mut handled_on_disk: Vec<String> = Vec::new();
        let mut to_excise: Vec<&FoundFence> = Vec::new();
        for (id, fence) in &cleanup.fences {
            let in_desired = desired_ids.iter().any(|d| d == id);
            let in_omit = omit_ids.iter().any(|o| o == id);
            if in_desired && !in_omit {
                continue; // desired walk already handled it
            }
            handled_on_disk.push(id.clone());
            match fence.kind {
                FenceKind::Overridden => actions.push(KeyAction::Overridden { key: id.clone() }),
                FenceKind::Managed => {
                    actions.push(KeyAction::Deleted {
                        key: id.clone(),
                        was: fence.inner.clone(),
                    });
                    if in_omit {
                        warnings.push(format!(
                            "yard:omit `{id}` removed the previously managed fence in .gitignore"
                        ));
                    }
                    to_excise.push(fence);
                }
            }
        }
        to_excise.sort_by_key(|f| std::cmp::Reverse(f.open_start));
        for fence in to_excise {
            contents = excise(&contents, fence.open_start, fence.close_end);
        }

        // Omits that matched no on-disk fence. In-desired omits are the
        // ordinary "user said no, yard would have emitted" case — report
        // Omitted with no warning. Out-of-desired omits are stale (no
        // managed target left to suppress); report Omitted *and* warn.
        for omit_id in &omit_ids {
            if handled_on_disk.iter().any(|h| h == omit_id) {
                continue;
            }
            actions.push(KeyAction::Omitted {
                key: omit_id.clone(),
            });
            if !desired_ids.iter().any(|d| d == omit_id) {
                warnings.push(format!(
                    "yard:omit `{omit_id}` does not match any currently-managed fence in .gitignore",
                ));
            }
        }

        // If after all reconciliation the file would be empty, signal
        // deletion. DESIGN.md doesn't spell out the empty-file case for
        // block-fenced files, but the spirit is symmetric with creation:
        // when nothing is left to manage and the user kept nothing of
        // their own, the file shouldn't exist.
        let contents_out = if contents.is_empty() {
            None
        } else {
            Some(contents)
        };

        Ok(ApplyOutcome {
            contents: contents_out,
            actions,
            warnings,
        })
    }
}

impl Adaptor for GitignoreAdaptor {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn path(&self, ctx: &RuntimeContext) -> PathBuf {
        GitignoreAdaptor::path(self, ctx)
    }

    fn plan(
        &self,
        contribs: Vec<(&'static str, Contribution)>,
        existing: Option<&str>,
        ctx: &RuntimeContext,
    ) -> Result<ApplyOutcome, PlanError> {
        let mine = contribs.into_iter().map(|(module, c)| match c {
            Contribution::Gitignore(g) => (module, g),
            // The engine routes by `Contribution::adaptor_id`; a mismatch is
            // a wiring bug, not user input.
            other => unreachable!(
                "engine routed {} contribution to gitignore adaptor",
                other.adaptor_id()
            ),
        });
        let desired = GitignoreDesired::from_contributions(mine)?;
        // Nothing to manage and no file on disk: signal "no file" so the
        // engine doesn't materialise an empty `.gitignore`. When a file
        // *does* exist, `apply` runs the full removal pass — stale
        // managed fences get spliced out, overridden ones reported, and
        // the file is deleted if nothing remains.
        if desired.fences.is_empty() && existing.is_none() {
            return Ok(ApplyOutcome {
                contents: None,
                actions: Vec::new(),
                warnings: Vec::new(),
            });
        }
        self.apply(&desired, existing, ctx).map_err(|e| PlanError::Parse {
            adaptor: Self::ID,
            source: Box::new(e),
        })
    }
}

/// Parse error surfaced when reconciling against a malformed `.gitignore`.
///
/// Per DESIGN.md: "A fence missing the id (or with mismatched open/close
/// ids) is a parse error: the file fails loud rather than letting yard
/// silently take or lose ownership." The struct carries the file path and a
/// 1-based line number so error messages point straight at the broken
/// marker; `kind` records which specific malformation was hit.
#[derive(Debug, thiserror::Error)]
#[error("{path}:{line}: {kind}", path = .path.display())]
pub struct GitignoreParseError {
    pub path: PathBuf,
    pub line: usize,
    pub kind: ParseErrorKind,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseErrorKind {
    #[error("yard fence is missing the required `id=<slug>`")]
    MissingId,
    #[error("yard fence id {0:?} is not a valid slug (allowed: [A-Za-z0-9_-]+)")]
    InvalidId(String),
    #[error("unknown yard fence kind {0:?}; expected `managed` or `overridden`")]
    UnknownKind(String),
    #[error("malformed yard fence marker: {0:?}")]
    MalformedMarker(String),
    #[error("yard fence open id={open:?} does not match close id={close:?}")]
    MismatchedIds { open: String, close: String },
    #[error("yard fence close id={0:?} has no matching open above it")]
    OrphanClose(String),
    #[error("yard fence id={0:?} was opened but never closed")]
    Unterminated(String),
}

fn render_inner(lines: &[String]) -> String {
    lines.join("\n")
}

fn render_block(id: &str, inner: &str) -> String {
    let open = format!("# >>> yard:managed id={id} >>>");
    let close = format!("# <<< yard:managed id={id} <<<");
    if inner.is_empty() {
        format!("{open}\n{close}\n")
    } else {
        format!("{open}\n{inner}\n{close}\n")
    }
}

fn append_block(contents: &str, id: &str, inner: &str) -> String {
    let block = render_block(id, inner);
    if contents.is_empty() {
        return block;
    }
    let mut out = contents.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&block);
    out
}

fn splice(s: &str, start: usize, end: usize, replacement: &str) -> String {
    let mut out = String::with_capacity(s.len() - (end - start) + replacement.len());
    out.push_str(&s[..start]);
    out.push_str(replacement);
    out.push_str(&s[end..]);
    out
}

/// Remove the byte range `[start, end)` and one adjacent blank-line
/// separator if there is one. `append_block` inserts a single blank line
/// before a freshly-appended block; on removal we eat that separator so
/// round-trip applies stay stable instead of accumulating blanks. The
/// separator can be above (the common case) or below (when the excised
/// block is at the very top of the file). Never eat more than one — any
/// blank lines beyond the immediate separator are user-authored spacing.
fn excise(s: &str, start: usize, end: usize) -> String {
    let bytes = s.as_bytes();
    // Blank line *above* — recognised as two consecutive `\n`s
    // immediately before `start`. The byte at `start - 1` is the line
    // ending of whatever's above; the byte at `start - 2` being `\n` too
    // means that line is empty (i.e., a blank separator).
    let trim_start = if start >= 2 && bytes[start - 1] == b'\n' && bytes[start - 2] == b'\n' {
        start - 1
    } else {
        start
    };
    // Blank line *below* — only consumed when the excise reaches the
    // top of the file (no content above to "own" the separator) or
    // when the excise immediately follows another excised block, i.e.
    // there's no content immediately above either. In both cases the
    // remaining file would otherwise start with `\n`.
    let nothing_above = trim_start == 0 || bytes[trim_start - 1] == b'\n';
    let trim_end = if nothing_above && trim_start == start && s[end..].starts_with('\n') {
        end + 1
    } else {
        end
    };
    let mut out = String::with_capacity(s.len() - (trim_end - trim_start));
    out.push_str(&s[..trim_start]);
    out.push_str(&s[trim_end..]);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FenceKind {
    Managed,
    Overridden,
}

#[derive(Debug, Clone)]
struct FoundFence {
    kind: FenceKind,
    /// Byte offset of the start of the open-marker line.
    open_start: usize,
    /// Byte offset just past the close-marker line's terminating `\n` (or
    /// end-of-string if the close marker had no trailing newline).
    close_end: usize,
    /// Joined inner lines, no trailing newline (matches `render_inner`).
    inner: String,
}

#[derive(Debug, Default)]
struct ParsedMarkers {
    fences: Vec<(String, FoundFence)>,
    omits: Vec<OmitLine>,
}

impl ParsedMarkers {
    fn find_fence(&self, id: &str) -> Option<&FoundFence> {
        self.fences.iter().find(|(k, _)| k == id).map(|(_, v)| v)
    }
}

#[derive(Debug, Clone)]
struct OmitLine {
    target: OmitTarget,
}

#[derive(Debug, Clone)]
enum OmitTarget {
    /// Argument was a well-formed slug — yard knows what it refers to and
    /// can compare against the adaptor's currently-managed ids.
    Slug(String),
    /// Argument was malformed (e.g. contained a `.`); surfaced as a
    /// warning. DESIGN.md: "the adaptor warns and otherwise ignores the
    /// line." Stored as the raw argument so the warning can echo it.
    Invalid(String),
}

/// Walk `content` for all yard fences and omit comments. Any line that
/// looks like a yard fence marker (`# >>> yard:` / `# <<< yard:` prefix)
/// but does not parse cleanly aborts the walk with a [`PendingParseError`]
/// carrying the 1-based line number and the specific [`ParseErrorKind`].
/// `# yard:omit <arg>` is non-fatal: a malformed argument produces an
/// [`OmitTarget::Invalid`] which the caller surfaces as a warning.
fn parse_markers(content: &str) -> Result<ParsedMarkers, PendingParseError> {
    let mut out = ParsedMarkers::default();
    let mut open: Option<OpenState> = None;
    let mut cursor = 0usize;

    for (idx, line) in content.split_inclusive('\n').enumerate() {
        let line_num = idx + 1;
        let line_start = cursor;
        let line_end = cursor + line.len();
        cursor = line_end;
        let trimmed = strip_eol(line);

        if let Some(attempt) = parse_marker(trimmed, MarkerDir::Open) {
            let marker = attempt.map_err(|kind| PendingParseError {
                line: line_num,
                kind,
            })?;
            if let Some(prev) = &open {
                // A second open before the previous closed — the previous
                // fence is unterminated.
                return Err(PendingParseError {
                    line: prev.open_line,
                    kind: ParseErrorKind::Unterminated(prev.id.clone()),
                });
            }
            open = Some(OpenState {
                kind: marker.kind,
                id: marker.id,
                open_start: line_start,
                open_end: line_end,
                open_line: line_num,
            });
        } else if let Some(attempt) = parse_marker(trimmed, MarkerDir::Close) {
            let marker = attempt.map_err(|kind| PendingParseError {
                line: line_num,
                kind,
            })?;
            let Some(state) = open.take() else {
                return Err(PendingParseError {
                    line: line_num,
                    kind: ParseErrorKind::OrphanClose(marker.id),
                });
            };
            if marker.kind != state.kind || marker.id != state.id {
                return Err(PendingParseError {
                    line: line_num,
                    kind: ParseErrorKind::MismatchedIds {
                        open: state.id,
                        close: marker.id,
                    },
                });
            }
            let inner_bytes = &content[state.open_end..line_start];
            let inner = inner_bytes.strip_suffix('\n').unwrap_or(inner_bytes);
            out.fences.push((
                state.id.clone(),
                FoundFence {
                    kind: state.kind,
                    open_start: state.open_start,
                    close_end: line_end,
                    inner: inner.to_string(),
                },
            ));
        } else if open.is_none() {
            // Omits are only recognised outside a fence; lines inside a
            // managed/overridden block are part of that block's payload.
            if let Some(target) = parse_omit(trimmed) {
                out.omits.push(OmitLine { target });
            }
        }
    }

    if let Some(state) = open {
        return Err(PendingParseError {
            line: state.open_line,
            kind: ParseErrorKind::Unterminated(state.id),
        });
    }

    Ok(out)
}

/// Recognise `# yard:omit <arg>` (with the standard `[A-Za-z0-9_.-]+`
/// charset from DESIGN.md — gitignore's notion of "valid" is the
/// fence-id slug, so anything with a dot becomes `Invalid` and surfaces
/// as a warning). Returns `None` when the line is not a yard:omit
/// comment at all.
fn parse_omit(line: &str) -> Option<OmitTarget> {
    let rest = line.strip_prefix("# yard:omit")?;
    let arg = rest.trim();
    if arg.is_empty() {
        return Some(OmitTarget::Invalid(String::new()));
    }
    if is_valid_slug(arg) {
        Some(OmitTarget::Slug(arg.to_string()))
    } else {
        Some(OmitTarget::Invalid(arg.to_string()))
    }
}

struct OpenState {
    kind: FenceKind,
    id: String,
    open_start: usize,
    open_end: usize,
    open_line: usize,
}

/// Parse-time error before a path has been attached. `parse_fences` is path-
/// agnostic; the adaptor stamps the path on as it lifts this into a
/// [`GitignoreParseError`].
struct PendingParseError {
    line: usize,
    kind: ParseErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerDir {
    Open,
    Close,
}

#[derive(Debug)]
struct Marker {
    kind: FenceKind,
    id: String,
}

/// Three-way recognition:
/// - `None`            — line is not a yard marker line at all (prefix mismatch).
/// - `Some(Ok(_))`     — well-formed yard marker.
/// - `Some(Err(_))`    — line started with the yard prefix but failed to parse.
///
/// Once the prefix matches, the line is committed to yard: any malformation
/// becomes a parse error rather than being silently treated as user content.
fn parse_marker(line: &str, dir: MarkerDir) -> Option<Result<Marker, ParseErrorKind>> {
    let (prefix, suffix) = match dir {
        MarkerDir::Open => ("# >>> yard:", " >>>"),
        MarkerDir::Close => ("# <<< yard:", " <<<"),
    };
    let rest = line.strip_prefix(prefix)?;
    let body = match rest.strip_suffix(suffix) {
        Some(b) => b,
        None => return Some(Err(ParseErrorKind::MalformedMarker(line.to_string()))),
    };
    let (kind_str, rest) = match body.split_once(' ') {
        Some(parts) => parts,
        None => return Some(Err(ParseErrorKind::MissingId)),
    };
    let kind = match kind_str {
        "managed" => FenceKind::Managed,
        "overridden" => FenceKind::Overridden,
        other => return Some(Err(ParseErrorKind::UnknownKind(other.to_string()))),
    };
    let id = match rest.strip_prefix("id=") {
        Some(id) => id,
        None => return Some(Err(ParseErrorKind::MissingId)),
    };
    if !is_valid_slug(id) {
        return Some(Err(ParseErrorKind::InvalidId(id.to_string())));
    }
    Some(Ok(Marker {
        kind,
        id: id.to_string(),
    }))
}

fn is_valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn strip_eol(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

#[cfg(test)]
mod tests {
    //! Add a new scenario: drop a directory under `fixtures/` containing
    //! `desired.ron` (+ optional `existing.gitignore`), then add a one-line
    //! `#[test]` below. `expected.gitignore` and `expected.actions` are
    //! generated on the first run with `UPDATE_GOLDENS=1`.

    use super::*;
    use crate::adaptors::test_harness::{ApplyHarness, run_apply_fixture};

    const HARNESS: ApplyHarness = ApplyHarness {
        fixtures_root: concat!(env!("CARGO_MANIFEST_DIR"), "/src/adaptors/gitignore/fixtures"),
        existing_filename: "existing.gitignore",
        expected_filename: "expected.gitignore",
    };

    fn run(name: &str) {
        run_apply_fixture::<GitignoreDesired, _>(&HARNESS, name, |d, e, r| {
            // Fixture inputs are all well-formed; a parse error here is a
            // bug in the fixture, not behaviour under test.
            GitignoreAdaptor
                .apply(d, e, r)
                .expect("fixture inputs should parse cleanly")
        });
    }

    #[test] fn create_fresh()                       { run("create_fresh"); }
    #[test] fn in_sync()                            { run("in_sync"); }
    #[test] fn update_in_place()                    { run("update_in_place"); }
    #[test] fn appends_when_no_fence()              { run("appends_when_no_fence"); }
    #[test] fn overridden_block()                   { run("overridden_block"); }
    #[test] fn multiple_fences_fresh()              { run("multiple_fences_fresh"); }
    #[test] fn multiple_fences_with_existing()      { run("multiple_fences_with_existing"); }
    #[test] fn removes_stale_managed_fence()        { run("removes_stale_managed_fence"); }
    #[test] fn overridden_stale_fence_preserved()   { run("overridden_stale_fence_preserved"); }
    #[test] fn omit_suppresses_emission()           { run("omit_suppresses_emission"); }
    #[test] fn stale_omit_warns()                   { run("stale_omit_warns"); }
    #[test] fn invalid_omit_warns()                 { run("invalid_omit_warns"); }
    #[test] fn mixed_retention_and_removal()        { run("mixed_retention_and_removal"); }
    #[test] fn multiple_stale_managed_fences()      { run("multiple_stale_managed_fences"); }
    #[test] fn omit_removes_existing_managed_fence() { run("omit_removes_existing_managed_fence"); }

    /// Deletion of the file itself is shape-incompatible with the
    /// golden-fixture harness (which always expects file contents). Keep
    /// this single case inline rather than growing a "no file" mode into
    /// the harness for one scenario.
    #[test]
    fn signals_deletion_when_no_content_remains() {
        let existing =
            "# >>> yard:managed id=defunct >>>\nbuild/\n# <<< yard:managed id=defunct <<<\n";
        let desired = GitignoreDesired::default();
        let runtime = RuntimeContext {
            workspace: std::path::Path::new("/tmp"),
            yard_version: env!("CARGO_PKG_VERSION"),
        };
        let outcome = GitignoreAdaptor.apply(&desired, Some(existing), &runtime).unwrap();
        assert_eq!(outcome.contents, None);
        assert_eq!(
            outcome.actions,
            vec![KeyAction::Deleted {
                key: "defunct".into(),
                was: "build/".into(),
            }],
        );
        assert!(outcome.warnings.is_empty());
    }

    /// Pure merge logic — different shape from `apply`, so it stays inline.
    #[test]
    fn merges_and_deduplicates_lines() {
        let merged = GitignoreDesired::from_contributions([
            (
                "mod_a",
                GitignoreContribution {
                    id: "standard-ignores".into(),
                    lines: vec!["build/".into(), "install/".into()],
                },
            ),
            (
                "mod_b",
                GitignoreContribution {
                    id: "standard-ignores".into(),
                    lines: vec!["install/".into(), "log/".into()],
                },
            ),
        ])
        .expect("gitignore merge is additive and cannot fail today");
        assert_eq!(merged.fences.len(), 1);
        assert_eq!(merged.fences[0].id, "standard-ignores");
        assert_eq!(merged.fences[0].lines, vec!["build/", "install/", "log/"]);
    }

    #[test]
    fn groups_contributions_by_id() {
        let merged = GitignoreDesired::from_contributions([
            (
                "mod_a",
                GitignoreContribution {
                    id: "a".into(),
                    lines: vec!["x".into()],
                },
            ),
            (
                "mod_b",
                GitignoreContribution {
                    id: "b".into(),
                    lines: vec!["y".into()],
                },
            ),
            (
                "mod_a",
                GitignoreContribution {
                    id: "a".into(),
                    lines: vec!["z".into()],
                },
            ),
        ])
        .expect("gitignore merge is additive and cannot fail today");
        assert_eq!(merged.fences.len(), 2);
        assert_eq!(merged.fences[0].id, "a");
        assert_eq!(merged.fences[0].lines, vec!["x", "z"]);
        assert_eq!(merged.fences[1].id, "b");
        assert_eq!(merged.fences[1].lines, vec!["y"]);
    }
}
