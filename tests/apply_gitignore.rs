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
