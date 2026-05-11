//! `.gitignore` adaptor — block-fencing marking strategy.
//!
//! yard owns the lines between `# >>> yard:managed >>>` and
//! `# <<< yard:managed <<<`. Everything outside the fence is the user's and
//! is preserved verbatim. If the fence is rewritten as `yard:frozen`, yard
//! leaves the entire block alone.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::RuntimeContext;
use crate::adaptors::{ApplyOutcome, KeyAction};

const FENCE_OPEN_MANAGED: &str = "# >>> yard:managed >>>";
const FENCE_CLOSE_MANAGED: &str = "# <<< yard:managed <<<";
const FENCE_OPEN_FROZEN: &str = "# >>> yard:frozen >>>";
const FENCE_CLOSE_FROZEN: &str = "# <<< yard:frozen <<<";

/// Key used in action reports for the single managed gitignore block.
const BLOCK_KEY: &str = ".gitignore:managed";

/// Fragment a module wants merged into the gitignore. `lines` are raw
/// gitignore patterns (e.g. `"build/"`); order is preserved on first sight,
/// duplicates across contributions are dropped on merge.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct GitignoreContribution {
    pub lines: Vec<String>,
}

/// Merged set of lines the gitignore adaptor will write inside the fence.
///
/// Lines are deduplicated but order is otherwise preserved: the first module
/// to mention a line decides where it appears, which gives deterministic
/// output (modules run in registry order).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct GitignoreDesired {
    pub lines: Vec<String>,
}

impl GitignoreDesired {
    /// Merge a sequence of contributions in iteration order. Duplicate lines
    /// after the first occurrence are dropped.
    pub fn from_contributions<I>(contribs: I) -> Self
    where
        I: IntoIterator<Item = GitignoreContribution>,
    {
        let mut lines: Vec<String> = Vec::new();
        for c in contribs {
            for line in c.lines {
                if !lines.iter().any(|l| l == &line) {
                    lines.push(line);
                }
            }
        }
        Self { lines }
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
        let desired_inner = render_inner(desired);

        let Some(content) = existing else {
            // No file yet — emit just the fence.
            return ApplyOutcome {
                contents: render_block(&desired_inner),
                actions: vec![KeyAction::Reemitted {
                    key: BLOCK_KEY.into(),
                    to: desired_inner,
                }],
            };
        };

        match find_fence(content) {
            Some(fence) if fence.frozen => ApplyOutcome {
                contents: content.to_string(),
                actions: vec![KeyAction::Frozen {
                    key: BLOCK_KEY.into(),
                }],
            },
            Some(fence) if fence.inner == desired_inner => ApplyOutcome {
                contents: content.to_string(),
                actions: vec![KeyAction::InSync {
                    key: BLOCK_KEY.into(),
                }],
            },
            Some(fence) => {
                let new_block = render_block(&desired_inner);
                let contents = format!("{}{}{}", fence.prefix, new_block, fence.suffix);
                ApplyOutcome {
                    contents,
                    actions: vec![KeyAction::Updated {
                        key: BLOCK_KEY.into(),
                        from: fence.inner.to_string(),
                        to: desired_inner,
                    }],
                }
            }
            None => {
                // No fence: append at the end, preserving user content with a
                // blank-line separator if needed.
                let mut new_contents = content.to_string();
                if !new_contents.is_empty() && !new_contents.ends_with('\n') {
                    new_contents.push('\n');
                }
                if !new_contents.is_empty() {
                    new_contents.push('\n');
                }
                new_contents.push_str(&render_block(&desired_inner));
                ApplyOutcome {
                    contents: new_contents,
                    actions: vec![KeyAction::Reemitted {
                        key: BLOCK_KEY.into(),
                        to: desired_inner,
                    }],
                }
            }
        }
    }
}

/// Lines yard wants inside the fence, joined with `\n` and no trailing newline.
fn render_inner(desired: &GitignoreDesired) -> String {
    desired.lines.join("\n")
}

/// Full fenced block (open marker, inner lines, close marker), terminated
/// with a final newline so it concatenates cleanly with whatever follows.
fn render_block(inner: &str) -> String {
    if inner.is_empty() {
        format!("{FENCE_OPEN_MANAGED}\n{FENCE_CLOSE_MANAGED}\n")
    } else {
        format!("{FENCE_OPEN_MANAGED}\n{inner}\n{FENCE_CLOSE_MANAGED}\n")
    }
}

#[derive(Debug)]
struct Fence<'a> {
    prefix: &'a str,
    inner: String,
    suffix: &'a str,
    frozen: bool,
}

/// Locate the first managed-or-frozen fence in `content`. Splits the string
/// into the bytes before the open-fence line, the inner lines between the
/// fence markers (joined with `\n`, no trailing newline), and the bytes after
/// the close-fence line (including any leading newline that followed it).
fn find_fence(content: &str) -> Option<Fence<'_>> {
    let mut open: Option<(usize, usize, bool)> = None; // (line_start, line_end, frozen)
    let mut cursor = 0usize;

    for line in content.split_inclusive('\n') {
        let line_start = cursor;
        let line_end = cursor + line.len();
        cursor = line_end;
        let trimmed = strip_eol(line);

        match open {
            None => {
                if trimmed == FENCE_OPEN_MANAGED {
                    open = Some((line_start, line_end, false));
                } else if trimmed == FENCE_OPEN_FROZEN {
                    open = Some((line_start, line_end, true));
                }
            }
            Some((open_start, open_end, frozen)) => {
                let close_marker = if frozen {
                    FENCE_CLOSE_FROZEN
                } else {
                    FENCE_CLOSE_MANAGED
                };
                if trimmed == close_marker {
                    let inner_bytes = &content[open_end..line_start];
                    // Drop the trailing '\n' (if any) so callers compare against
                    // the same shape `render_inner` produces.
                    let inner = inner_bytes.strip_suffix('\n').unwrap_or(inner_bytes);
                    return Some(Fence {
                        prefix: &content[..open_start],
                        inner: inner.to_string(),
                        suffix: &content[line_end..],
                        frozen,
                    });
                }
            }
        }
    }

    None
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

    #[test] fn create_fresh()           { run("create_fresh"); }
    #[test] fn in_sync()                { run("in_sync"); }
    #[test] fn update_in_place()        { run("update_in_place"); }
    #[test] fn appends_when_no_fence()  { run("appends_when_no_fence"); }
    #[test] fn frozen_block()           { run("frozen_block"); }

    /// Pure merge logic — different shape from `apply`, so it stays inline.
    #[test]
    fn merges_and_deduplicates_lines() {
        let merged = GitignoreDesired::from_contributions([
            GitignoreContribution {
                lines: vec!["build/".into(), "install/".into()],
            },
            GitignoreContribution {
                lines: vec!["install/".into(), "log/".into()],
            },
        ]);
        assert_eq!(merged.lines, vec!["build/", "install/", "log/"]);
    }
}
