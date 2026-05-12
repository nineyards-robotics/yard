//! Shared test harness for adaptor `apply` fixtures.
//!
//! Each adaptor defines an `ApplyHarness` constant and writes one
//! `#[test] fn` per scenario; this module owns the read-input → call-apply →
//! golden-diff loop. Cost per new fixture is a directory + one `#[test]`.

use std::fs;
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::RuntimeContext;
use crate::adaptors::{ApplyOutcome, KeyAction};
use crate::test_support::assert_golden;

/// Per-adaptor configuration. Filenames mirror the real on-disk shape so
/// fixture files double as documentation (`existing.gitignore` reads as a
/// real `.gitignore`, etc.).
pub struct ApplyHarness {
    pub fixtures_root: &'static str,
    pub existing_filename: &'static str,
    pub expected_filename: &'static str,
}

/// Run one apply-shaped fixture and golden-diff both outputs.
#[track_caller]
pub fn run_apply_fixture<D, F>(harness: &ApplyHarness, scenario: &str, apply: F)
where
    D: DeserializeOwned,
    F: FnOnce(&D, Option<&str>, &RuntimeContext) -> ApplyOutcome,
{
    let dir = Path::new(harness.fixtures_root).join(scenario);

    let raw = fs::read_to_string(dir.join("desired.ron"))
        .unwrap_or_else(|e| panic!("read desired.ron in {}: {e}", dir.display()));
    let desired: D = ron::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse desired.ron in {}: {e}", dir.display()));

    let existing_path = dir.join(harness.existing_filename);
    let existing = existing_path
        .exists()
        .then(|| fs::read_to_string(&existing_path).unwrap());

    let runtime = RuntimeContext {
        workspace: &dir,
        yard_version: env!("CARGO_PKG_VERSION"),
    };
    let outcome = apply(&desired, existing.as_deref(), &runtime);

    assert_golden(&dir.join(harness.expected_filename), &outcome.contents);
    assert_golden(&dir.join("expected.actions"), &format_actions(&outcome.actions));
}

/// Render a `Vec<KeyAction>` as the canonical `expected.actions` form: one
/// `Kind key` per line. Inner payloads are dropped — they're derivable from
/// the file output and would just duplicate surface to maintain.
pub fn format_actions(actions: &[KeyAction]) -> String {
    let mut out = String::new();
    for a in actions {
        let (kind, key) = match a {
            KeyAction::InSync { key } => ("InSync", key),
            KeyAction::Updated { key, .. } => ("Updated", key),
            KeyAction::Reemitted { key, .. } => ("Reemitted", key),
            KeyAction::Overridden { key } => ("Overridden", key),
        };
        out.push_str(&format!("{kind} {key}\n"));
    }
    out
}
