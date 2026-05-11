//! Universal `#[cfg(test)]` helper.
//!
//! Adaptor- and module-specific harnesses live next to their respective
//! modules (`crate::adaptors::test_harness`, `crate::modules::test_harness`)
//! and call into `assert_golden` from here. Anything that knows about
//! `ApplyOutcome`, `KeyAction`, `Contribution`, etc. belongs there, not here.

use std::fs;
use std::path::Path;

/// Diff `actual` against the contents of the file at `path`. With
/// `UPDATE_GOLDENS=1` set, rewrite the file instead — review the diff in
/// git before committing.
#[track_caller]
pub fn assert_golden(path: &Path, actual: &str) {
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("could not create {}: {e}", parent.display()));
        }
        fs::write(path, actual)
            .unwrap_or_else(|e| panic!("could not write golden {}: {e}", path.display()));
        return;
    }

    let expected = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "could not read golden {} (run with UPDATE_GOLDENS=1 to create): {e}",
            path.display()
        )
    });

    pretty_assertions::assert_eq!(
        actual,
        expected.as_str(),
        "golden mismatch at {}",
        path.display()
    );
}
