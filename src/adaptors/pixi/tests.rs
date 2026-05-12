//! Tests for the pixi adaptor.
//!
//! Fixture-driven scenarios cover every state in DESIGN.md's per-key
//! classification table plus removal and file-preservation behaviour.
//! Inline tests cover shapes the fixture harness doesn't naturally
//! express: the `contents: None` signal, merge rules, and parse errors.

use std::collections::BTreeMap;
use std::path::Path;

use super::*;
use crate::RuntimeContext;
use crate::adaptors::test_harness::{ApplyHarness, run_apply_fixture};
use crate::adaptors::{Adaptor, ApplyOutcome};

const HARNESS: ApplyHarness = ApplyHarness {
    fixtures_root: concat!(env!("CARGO_MANIFEST_DIR"), "/src/adaptors/pixi/fixtures"),
    existing_filename: "existing.pixi.toml",
    expected_filename: "expected.pixi.toml",
};

fn run(name: &str) {
    run_apply_fixture::<PixiDesired, _>(&HARNESS, name, |d, e, r| {
        // Fixture inputs are all well-formed; a parse error here is a bug
        // in the fixture, not behaviour under test.
        PixiAdaptor
            .apply(d, e, r)
            .expect("fixture inputs should parse cleanly")
    });
}

// ── Fixture scenarios ────────────────────────────────────────────────
// Each fixture lives in `fixtures/<name>/`. Adding a scenario: drop the
// directory + add a one-liner here.

#[test] fn create_fresh_minimal()                  { run("create_fresh_minimal"); }
#[test] fn create_fresh_with_dependencies()        { run("create_fresh_with_dependencies"); }

#[test] fn scalar_in_sync()                        { run("scalar_in_sync"); }
#[test] fn scalar_updated()                        { run("scalar_updated"); }
#[test] fn scalar_conflict()                       { run("scalar_conflict"); }
#[test] fn scalar_overridden()                     { run("scalar_overridden"); }
#[test] fn scalar_omitted()                        { run("scalar_omitted"); }
#[test] fn scalar_unmarked_attaches_marker()       { run("scalar_unmarked_attaches_marker"); }
#[test] fn scalar_missing_reemitted()              { run("scalar_missing_reemitted"); }
#[test] fn scalar_unparsable_default_reemits()     { run("scalar_unparsable_default_reemits"); }
#[test] fn scalar_reemitted_creates_section()      { run("scalar_reemitted_creates_section"); }

#[test] fn array_in_sync()                         { run("array_in_sync"); }
#[test] fn array_updated()                         { run("array_updated"); }
#[test] fn array_user_added_preserved()            { run("array_user_added_preserved"); }
#[test] fn array_user_added_reorders()             { run("array_user_added_reorders"); }
#[test] fn array_user_removed_is_conflict()        { run("array_user_removed_is_conflict"); }
#[test] fn array_overridden()                      { run("array_overridden"); }
#[test] fn array_desired_drops_element()           { run("array_desired_drops_element"); }
#[test] fn array_unmarked_attaches_marker()        { run("array_unmarked_attaches_marker"); }
#[test] fn array_missing_reemitted()               { run("array_missing_reemitted"); }
#[test] fn array_omitted()                         { run("array_omitted"); }

#[test] fn dependency_added()                      { run("dependency_added"); }
#[test] fn dependency_in_sync()                    { run("dependency_in_sync"); }
#[test] fn dependency_updated()                    { run("dependency_updated"); }
#[test] fn dependency_conflict()                   { run("dependency_conflict"); }
#[test] fn dependency_overridden()                 { run("dependency_overridden"); }
#[test] fn dependency_omitted()                    { run("dependency_omitted"); }
#[test] fn dependency_unmarked_attaches_marker()   { run("dependency_unmarked_attaches_marker"); }
#[test] fn unmanaged_dependency_preserved()        { run("unmanaged_dependency_preserved"); }

#[test] fn removal_in_sync_deletes()               { run("removal_in_sync_deletes"); }
#[test] fn removal_dependency_in_sync_deletes()    { run("removal_dependency_in_sync_deletes"); }
#[test] fn removal_array_in_sync_deletes()         { run("removal_array_in_sync_deletes"); }
#[test] fn removal_overridden_left_alone()         { run("removal_overridden_left_alone"); }
#[test] fn removal_stale_omit_warns()              { run("removal_stale_omit_warns"); }
#[test] fn removal_conflict_blocks()               { run("removal_conflict_blocks"); }

#[test] fn invalid_omit_warns()                    { run("invalid_omit_warns"); }
#[test] fn mixed_states()                          { run("mixed_states"); }
#[test] fn preserves_user_sections_and_comments()  { run("preserves_user_sections_and_comments"); }

// ── Inline tests ─────────────────────────────────────────────────────

fn dummy_runtime() -> RuntimeContext<'static> {
    RuntimeContext {
        // Paths are only used to label errors; the adaptor never touches
        // the filesystem.
        workspace: Path::new("/tmp/yard-pixi-test"),
        yard_version: env!("CARGO_PKG_VERSION"),
    }
}

/// With no contributions and no existing file, the adaptor signals
/// `contents: None` so the engine doesn't materialise an empty
/// `pixi.toml`. Mirrors gitignore's same behaviour.
#[test]
fn no_contributions_and_no_file_signals_no_file() {
    let outcome = PixiAdaptor
        .plan(Vec::new(), None, &dummy_runtime())
        .expect("plan should succeed");
    assert!(
        outcome.contents.is_none(),
        "no contributions + no file should yield contents: None, got {:?}",
        outcome.contents,
    );
    assert!(outcome.actions.is_empty(), "no contributions ⇒ no actions");
    assert!(outcome.warnings.is_empty(), "no contributions ⇒ no warnings");
}

/// `Some("") + empty desired` is distinct from `None + empty desired`: a file
/// exists on disk (it's just empty) and the adaptor must leave it alone.
/// The `contents: None` signal is reserved for "no file should exist"; an
/// existing empty file passes through unchanged so yard never silently
/// deletes a file the user (or another tool) deliberately created.
#[test]
fn empty_existing_with_empty_desired_passes_through_unchanged() {
    let outcome = PixiAdaptor
        .plan(Vec::new(), Some(""), &dummy_runtime())
        .expect("plan should succeed");
    assert_eq!(
        outcome.contents.as_deref(),
        Some(""),
        "empty file + no contributions must passthrough as Some(\"\"), got {:?}",
        outcome.contents,
    );
    assert!(outcome.actions.is_empty(), "no managed keys ⇒ no actions");
    assert!(outcome.warnings.is_empty(), "no managed keys ⇒ no warnings");
}

// ---- Merge ----

#[test]
fn merge_scalar_agreement() {
    let merged = PixiDesired::from_contributions([
        (
            "mod_a",
            PixiContribution {
                workspace_name: Some("myws".into()),
                ..Default::default()
            },
        ),
        (
            "mod_b",
            PixiContribution {
                workspace_name: Some("myws".into()),
                ..Default::default()
            },
        ),
    ])
    .expect("matching scalars merge");
    assert_eq!(merged.workspace_name.as_deref(), Some("myws"));
}

#[test]
fn merge_scalar_conflict_errors() {
    let err = PixiDesired::from_contributions([
        (
            "mod_a",
            PixiContribution {
                workspace_name: Some("a".into()),
                ..Default::default()
            },
        ),
        (
            "mod_b",
            PixiContribution {
                workspace_name: Some("b".into()),
                ..Default::default()
            },
        ),
    ])
    .expect_err("disagreeing scalars must error");
    assert_eq!(err.key, "workspace.name");
    assert!(
        err.modules.iter().any(|m| *m == "mod_a") && err.modules.iter().any(|m| *m == "mod_b"),
        "error names both modules, got {:?}",
        err.modules,
    );
}

#[test]
fn merge_channels_unions_with_dedup() {
    let merged = PixiDesired::from_contributions([
        (
            "mod_a",
            PixiContribution {
                channels: vec!["conda-forge".into(), "robostack-jazzy".into()],
                ..Default::default()
            },
        ),
        (
            "mod_b",
            PixiContribution {
                channels: vec!["robostack-jazzy".into(), "extras".into()],
                ..Default::default()
            },
        ),
    ])
    .expect("array merges are additive");
    assert_eq!(
        merged.channels,
        vec!["conda-forge", "robostack-jazzy", "extras"]
    );
}

#[test]
fn merge_dependencies_disjoint() {
    let merged = PixiDesired::from_contributions([
        (
            "mod_a",
            PixiContribution {
                dependencies: BTreeMap::from([("python".into(), "3.11".into())]),
                ..Default::default()
            },
        ),
        (
            "mod_b",
            PixiContribution {
                dependencies: BTreeMap::from([("numpy".into(), ">=1.20".into())]),
                ..Default::default()
            },
        ),
    ])
    .expect("disjoint map keys merge");
    assert_eq!(
        merged.dependencies.get("python").map(String::as_str),
        Some("3.11"),
    );
    assert_eq!(
        merged.dependencies.get("numpy").map(String::as_str),
        Some(">=1.20"),
    );
}

#[test]
fn merge_dependencies_conflict_errors() {
    let err = PixiDesired::from_contributions([
        (
            "mod_a",
            PixiContribution {
                dependencies: BTreeMap::from([("python".into(), "3.11".into())]),
                ..Default::default()
            },
        ),
        (
            "mod_b",
            PixiContribution {
                dependencies: BTreeMap::from([("python".into(), "3.12".into())]),
                ..Default::default()
            },
        ),
    ])
    .expect_err("same map key with differing values must error");
    assert_eq!(err.key, "dependencies.python");
}

// ---- Parse errors ----

fn apply_str(existing: &str, desired: PixiDesired) -> Result<ApplyOutcome, PixiParseError> {
    PixiAdaptor.apply(&desired, Some(existing), &dummy_runtime())
}

#[test]
fn parse_error_invalid_toml() {
    let err = apply_str("[workspace\nname = \"x\"\n", PixiDesired::default())
        .expect_err("invalid TOML should fail parse");
    assert!(
        matches!(err.kind, PixiParseErrorKind::InvalidToml(_)),
        "got {:?}",
        err.kind,
    );
}

#[test]
fn parse_error_managed_missing_default() {
    // `# yard:managed` without `default=` is the central malformation the
    // marker scheme must reject — without `default=` yard has no record of
    // what value to compare against, so silently treating the marker as
    // user content (or as a fresh marker) would be a hidden bug.
    let existing = "[workspace]\nname = \"x\"  # yard:managed\n";
    let err = apply_str(
        existing,
        PixiDesired {
            workspace_name: Some("x".into()),
            ..Default::default()
        },
    )
    .expect_err("marker missing default= must fail");
    assert!(
        matches!(err.kind, PixiParseErrorKind::ManagedMissingDefault { .. }),
        "got {:?}",
        err.kind,
    );
}

#[test]
fn parse_error_default_shape_mismatch() {
    // Scalar key with an array-shaped `default=` — yard can't compare a
    // string against an array, so the file fails loud rather than silently
    // re-classifying.
    let existing = "[workspace]\nname = \"x\"  # yard:managed default=[\"x\"]\n";
    let err = apply_str(
        existing,
        PixiDesired {
            workspace_name: Some("x".into()),
            ..Default::default()
        },
    )
    .expect_err("scalar key with array default= must fail");
    assert!(
        matches!(err.kind, PixiParseErrorKind::DefaultShapeMismatch { .. }),
        "got {:?}",
        err.kind,
    );
}
