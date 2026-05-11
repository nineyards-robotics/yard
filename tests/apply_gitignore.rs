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
        "# >>> yard:managed >>>\nbuild/\ninstall/\nlog/\n# <<< yard:managed <<<\n"
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

# >>> yard:managed >>>
build/
install/
log/
# <<< yard:managed <<<
"
    );

    // Round-trip: a follow-up run leaves user content alone too.
    yard_apply(ws.path()).success();
    assert_eq!(read_gitignore(ws.path()), contents);
}

#[test]
fn frozen_block_is_not_rewritten() {
    let ws = TempDir::new().unwrap();
    write_yard_toml(ws.path());

    let frozen = "\
# >>> yard:frozen >>>
build/
# <<< yard:frozen <<<
";
    fs::write(ws.path().join(".gitignore"), frozen).unwrap();

    yard_apply(ws.path())
        .success()
        .stdout(contains("frozen"));

    assert_eq!(read_gitignore(ws.path()), frozen);
}

#[test]
fn apply_fails_when_yard_toml_missing() {
    let ws = TempDir::new().unwrap();
    yard_apply(ws.path())
        .failure()
        .stderr(contains("yard.toml"));
}
