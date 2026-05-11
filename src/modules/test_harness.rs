//! Shared test harness for module `contribute` fixtures.
//!
//! Each module defines a `ModuleHarness` constant and writes one
//! `#[test] fn` per scenario. The fixture format is symmetric with the
//! adaptor harness: a real `yard.toml` snippet on input, the serialized
//! `Vec<Contribution>` on output (RON, since contributions are arbitrary
//! Rust enums and RON renders them naturally).

use std::fs;
use std::path::Path;

use ron::ser::PrettyConfig;

use crate::test_support::assert_golden;
use crate::{Contribution, YardConfig};

pub struct ModuleHarness {
    pub fixtures_root: &'static str,
}

#[track_caller]
pub fn run_module_fixture<F>(harness: &ModuleHarness, scenario: &str, contribute: F)
where
    F: FnOnce(&YardConfig) -> Vec<Contribution>,
{
    let dir = Path::new(harness.fixtures_root).join(scenario);

    let raw = fs::read_to_string(dir.join("yard.toml"))
        .unwrap_or_else(|e| panic!("read yard.toml in {}: {e}", dir.display()));
    let config = YardConfig::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse yard.toml in {}: {e}", dir.display()));

    let contribs = contribute(&config);
    let mut serialized = ron::ser::to_string_pretty(
        &contribs,
        PrettyConfig::default().struct_names(true),
    )
    .unwrap_or_else(|e| panic!("serialize contributions: {e}"));
    if !serialized.ends_with('\n') {
        serialized.push('\n');
    }

    assert_golden(&dir.join("expected.contributions.ron"), &serialized);
}
