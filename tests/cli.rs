//! End-to-end CLI smoke tests.
//!
//! These exercise the binary the same way a user would. Real verb behaviour
//! lands in later steps; for now we just pin down the surface so future
//! changes that accidentally break it fail loudly.

use assert_cmd::Command;
use predicates::str::contains;

fn yard() -> Command {
    Command::cargo_bin("yard").expect("yard binary should build")
}

#[test]
fn help_lists_subcommands() {
    yard()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("init"))
        .stdout(contains("apply"));
}

#[test]
fn version_flag_prints_a_version() {
    yard()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains("yard"));
}

#[test]
fn unknown_subcommand_fails() {
    yard().arg("definitely-not-a-real-verb").assert().failure();
}
