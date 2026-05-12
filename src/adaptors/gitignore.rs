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

use crate::RuntimeContext;
use crate::adaptors::{ApplyOutcome, KeyAction};

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
    pub fn from_contributions<I>(contribs: I) -> Self
    where
        I: IntoIterator<Item = GitignoreContribution>,
    {
        let mut fences: Vec<DesiredFence> = Vec::new();
        for c in contribs {
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
        Self { fences }
    }
}

pub struct GitignoreAdaptor;

impl GitignoreAdaptor {
    pub fn path(&self, ctx: &RuntimeContext) -> PathBuf {
        ctx.workspace.join(".gitignore")
    }

    pub fn apply(
        &self,
        desired: &GitignoreDesired,
        existing: Option<&str>,
        _ctx: &RuntimeContext,
    ) -> ApplyOutcome {
        let mut contents = existing.unwrap_or("").to_string();
        let mut actions = Vec::new();

        for fence in &desired.fences {
            // Re-parse for each fence: a previous splice/append may have
            // shifted byte offsets. Cheap for the file sizes we deal with
            // and keeps the splice math correct without offset bookkeeping.
            let parsed = parse_fences(&contents);
            let inner = render_inner(&fence.lines);
            match parsed.find(&fence.id) {
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

        ApplyOutcome { contents, actions }
    }
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
struct ParsedFences {
    fences: Vec<(String, FoundFence)>,
}

impl ParsedFences {
    fn find(&self, id: &str) -> Option<&FoundFence> {
        self.fences.iter().find(|(k, _)| k == id).map(|(_, v)| v)
    }
}

/// Walk `content` for all yard fences. Malformed markers (missing `id=`,
/// mismatched open/close, unknown kind) are silently ignored for now; a
/// future change should surface them through an error channel — see DESIGN.md
/// "A fence missing the id ... is a parse error".
fn parse_fences(content: &str) -> ParsedFences {
    let mut out = ParsedFences::default();
    let mut open: Option<(FenceKind, String, usize, usize)> = None;
    let mut cursor = 0usize;

    for line in content.split_inclusive('\n') {
        let line_start = cursor;
        let line_end = cursor + line.len();
        cursor = line_end;
        let trimmed = strip_eol(line);

        match &open {
            None => {
                if let Some(marker) = parse_marker(trimmed, MarkerDir::Open) {
                    open = Some((marker.kind, marker.id, line_start, line_end));
                }
            }
            Some((kind, id, open_start, open_end)) => {
                if let Some(marker) = parse_marker(trimmed, MarkerDir::Close)
                    && marker.kind == *kind
                    && marker.id == *id
                {
                    let inner_bytes = &content[*open_end..line_start];
                    let inner = inner_bytes.strip_suffix('\n').unwrap_or(inner_bytes);
                    out.fences.push((
                        id.clone(),
                        FoundFence {
                            kind: *kind,
                            open_start: *open_start,
                            close_end: line_end,
                            inner: inner.to_string(),
                        },
                    ));
                    open = None;
                }
                // Mismatched/other close: leave `open` set and keep scanning;
                // the next matching close (or end-of-file) terminates.
            }
        }
    }

    out
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

fn parse_marker(line: &str, dir: MarkerDir) -> Option<Marker> {
    let (prefix, suffix) = match dir {
        MarkerDir::Open => ("# >>> yard:", " >>>"),
        MarkerDir::Close => ("# <<< yard:", " <<<"),
    };
    let body = line.strip_prefix(prefix)?.strip_suffix(suffix)?;
    let (kind_str, rest) = body.split_once(' ')?;
    let kind = match kind_str {
        "managed" => FenceKind::Managed,
        "overridden" => FenceKind::Overridden,
        _ => return None,
    };
    let id = rest.strip_prefix("id=")?;
    if !is_valid_slug(id) {
        return None;
    }
    Some(Marker {
        kind,
        id: id.to_string(),
    })
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
            GitignoreAdaptor.apply(d, e, r)
        });
    }

    #[test] fn create_fresh()                    { run("create_fresh"); }
    #[test] fn in_sync()                         { run("in_sync"); }
    #[test] fn update_in_place()                 { run("update_in_place"); }
    #[test] fn appends_when_no_fence()           { run("appends_when_no_fence"); }
    #[test] fn overridden_block()                { run("overridden_block"); }
    #[test] fn multiple_fences_fresh()           { run("multiple_fences_fresh"); }
    #[test] fn multiple_fences_with_existing()   { run("multiple_fences_with_existing"); }

    /// Pure merge logic — different shape from `apply`, so it stays inline.
    #[test]
    fn merges_and_deduplicates_lines() {
        let merged = GitignoreDesired::from_contributions([
            GitignoreContribution {
                id: "standard-ignores".into(),
                lines: vec!["build/".into(), "install/".into()],
            },
            GitignoreContribution {
                id: "standard-ignores".into(),
                lines: vec!["install/".into(), "log/".into()],
            },
        ]);
        assert_eq!(merged.fences.len(), 1);
        assert_eq!(merged.fences[0].id, "standard-ignores");
        assert_eq!(merged.fences[0].lines, vec!["build/", "install/", "log/"]);
    }

    #[test]
    fn groups_contributions_by_id() {
        let merged = GitignoreDesired::from_contributions([
            GitignoreContribution {
                id: "a".into(),
                lines: vec!["x".into()],
            },
            GitignoreContribution {
                id: "b".into(),
                lines: vec!["y".into()],
            },
            GitignoreContribution {
                id: "a".into(),
                lines: vec!["z".into()],
            },
        ]);
        assert_eq!(merged.fences.len(), 2);
        assert_eq!(merged.fences[0].id, "a");
        assert_eq!(merged.fences[0].lines, vec!["x", "z"]);
        assert_eq!(merged.fences[1].id, "b");
        assert_eq!(merged.fences[1].lines, vec!["y"]);
    }
}
