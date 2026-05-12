//! End-to-end test for the gitignore vertical slice.
//!
//! Each test spins up a fresh tempdir as the workspace, writes a minimal
//! `yard.toml`, runs the binary, and inspects the resulting `.gitignore`.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

const MINIMAL_YARD_TOML: &str = "ros_distro = \"jazzy\"\n";

fn yard_apply(workspace: &Path) -> assert_cmd::assert::Assert {
    Command::cargo_bin("yard")
        .expect("yard binary should build")
        .current_dir(workspace)
        .arg("apply")
        .assert()
}

fn write_yard_toml(workspace: &Path) {
    fs::write(workspace.join("yard.toml"), MINIMAL_YARD_TOML).unwrap();
}

fn read_gitignore(workspace: &Path) -> String {
    fs::read_to_string(workspace.join(".gitignore")).unwrap()
}

#[test]
fn creates_gitignore_with_managed_block() {
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    yard_apply(ws.path()).success();

    let contents = read_gitignore(ws.path());
    assert_eq!(
        contents,
        "# >>> yard:managed id=standard-ignores >>>\n\
         build/\n\
         install/\n\
         log/\n\
         # <<< yard:managed id=standard-ignores <<<\n"
    );
}

#[test]
fn second_run_is_idempotent_and_reports_in_sync() {
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    yard_apply(ws.path()).success();
    let first = read_gitignore(ws.path());

    yard_apply(ws.path())
        .success()
        .stdout(contains("in sync"));

    let second = read_gitignore(ws.path());
    assert_eq!(first, second);
}

#[test]
fn preserves_user_lines_outside_fence() {
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    // User wrote their own gitignore before running yard.
    fs::write(
        ws.path().join(".gitignore"),
        "# personal\nsecret.env\n.idea/\n",
    )
    .unwrap();

    yard_apply(ws.path()).success();

    let contents = read_gitignore(ws.path());
    assert_eq!(
        contents,
        "\
# personal
secret.env
.idea/

# >>> yard:managed id=standard-ignores >>>
build/
install/
log/
# <<< yard:managed id=standard-ignores <<<
"
    );

    // Round-trip: a follow-up run leaves user content alone too.
    yard_apply(ws.path()).success();
    assert_eq!(read_gitignore(ws.path()), contents);
}

#[test]
fn overridden_block_is_not_rewritten() {
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    let overridden = "\
# >>> yard:overridden id=standard-ignores >>>
build/
# <<< yard:overridden id=standard-ignores <<<
";
    fs::write(ws.path().join(".gitignore"), overridden).unwrap();

    yard_apply(ws.path())
        .success()
        .stdout(contains("overridden"));

    assert_eq!(read_gitignore(ws.path()), overridden);
}

#[test]
fn apply_errors_on_managed_fence_without_id() {
    // Per DESIGN.md: "A fence missing the id ... is a parse error: the file
    // fails loud rather than letting yard silently take or lose ownership."
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    let malformed = "# >>> yard:managed >>>\nbuild/\n# <<< yard:managed <<<\n";
    fs::write(ws.path().join(".gitignore"), malformed).unwrap();

    yard_apply(ws.path())
        .failure()
        .stderr(contains(".gitignore"));

    // File must be untouched — no half-applied state.
    assert_eq!(read_gitignore(ws.path()), malformed);
}

#[test]
fn apply_errors_on_mismatched_fence_ids() {
    // Open says id=foo, close says id=bar — per DESIGN.md this is the same
    // parse-error class as a missing id.
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    let mismatched =
        "# >>> yard:managed id=foo >>>\nbuild/\n# <<< yard:managed id=bar <<<\n";
    fs::write(ws.path().join(".gitignore"), mismatched).unwrap();

    yard_apply(ws.path())
        .failure()
        .stderr(contains(".gitignore"));

    assert_eq!(read_gitignore(ws.path()), mismatched);
}

#[test]
fn apply_fails_when_yard_toml_missing() {
    let ws = TempDir::new().unwrap();
    yard_apply(ws.path())
        .failure()
        .stderr(contains("yard.toml"));
}

#[test]
fn omit_suppresses_managed_fence() {
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    // User opted out of the standard-ignores fence; yard must not write it.
    let initial = "# yard:omit standard-ignores\n.env\n";
    fs::write(ws.path().join(".gitignore"), initial).unwrap();

    yard_apply(ws.path())
        .success()
        .stdout(contains("omitted"));

    // Unchanged: the omit comment and the user's own ignore remain, no fence appended.
    assert_eq!(read_gitignore(ws.path()), initial);
}

#[test]
fn deletes_gitignore_when_only_yard_content_is_removed() {
    // ros_workspace currently always emits standard-ignores, so we simulate
    // "yard no longer wants this fence" by hand-rolling a stale managed
    // fence into a .gitignore whose id doesn't match anything yard emits.
    // After apply, that fence should be spliced out — and since nothing else
    // is in the file, the file itself should be deleted (per the design's
    // creation/removal symmetry).
    //
    // We also need the standard-ignores fence to land somewhere else, so
    // we point that at a second file by having yard create it fresh.
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    let stale = "# >>> yard:managed id=defunct >>>\nold/\n# <<< yard:managed id=defunct <<<\n";
    fs::write(ws.path().join(".gitignore"), stale).unwrap();

    yard_apply(ws.path())
        .success()
        .stdout(contains("deleted"));

    // The defunct fence was the only content, so the file is gone — yard
    // does not leave an empty .gitignore behind. The standard-ignores
    // fence still gets emitted into a freshly-created file though.
    let contents = read_gitignore(ws.path());
    assert_eq!(
        contents,
        "# >>> yard:managed id=standard-ignores >>>\n\
         build/\n\
         install/\n\
         log/\n\
         # <<< yard:managed id=standard-ignores <<<\n",
        "expected only the standard-ignores fence to remain"
    );
}

#[test]
fn second_apply_after_removal_is_idempotent() {
    // Removal must be stable: an apply that deletes a stale fence must
    // not re-report the deletion on the next run. Mirror of the
    // creation-side `second_run_is_idempotent_and_reports_in_sync`.
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    let stale =
        "# >>> yard:managed id=defunct >>>\nold/\n# <<< yard:managed id=defunct <<<\n";
    fs::write(ws.path().join(".gitignore"), stale).unwrap();

    yard_apply(ws.path())
        .success()
        .stdout(contains("deleted"));
    let after_first = read_gitignore(ws.path());

    let assert = yard_apply(ws.path()).success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        !stdout.contains("deleted"),
        "second apply should not re-delete; stdout was:\n{stdout}"
    );
    assert_eq!(read_gitignore(ws.path()), after_first);
}

#[test]
fn omit_with_existing_managed_fence_removes_it() {
    // User had yard's previously emitted fence on disk, then added a
    // `yard:omit` to take back control. yard cleans up its own fence and
    // warns so the user knows the omit took effect.
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    yard_apply(ws.path()).success();
    let with_omit = format!(
        "# yard:omit standard-ignores\n{}",
        read_gitignore(ws.path())
    );
    fs::write(ws.path().join(".gitignore"), &with_omit).unwrap();

    yard_apply(ws.path())
        .success()
        .stdout(contains("deleted"))
        .stderr(contains("warning"))
        .stderr(contains("standard-ignores"));

    let after = read_gitignore(ws.path());
    assert!(
        !after.contains("yard:managed"),
        "managed fence should have been removed: {after}"
    );
    assert!(
        after.contains("yard:omit standard-ignores"),
        "user's omit comment should be preserved: {after}"
    );
}

#[test]
fn stale_omit_emits_warning() {
    // User left an omit for a fence id no module is currently emitting.
    // yard does not block; it warns on stderr and otherwise leaves the
    // line alone.
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    let initial = "# yard:omit not-a-real-fence\n.env\n";
    fs::write(ws.path().join(".gitignore"), initial).unwrap();

    yard_apply(ws.path())
        .success()
        .stderr(contains("not-a-real-fence"))
        .stderr(contains("warning"));
}
