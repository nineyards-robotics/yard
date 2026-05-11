//! Integration tests for `yard.toml` parsing.
//!
//! Fixtures under `tests/fixtures/config/` are real files in the shape a user
//! would write. Each test loads one through the public `from_path` API, so the
//! fixtures double as documentation of supported (and rejected) shapes.

use std::path::PathBuf;

use yard::{ConfigError, YardConfig};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/config")
        .join(name)
}

#[test]
fn parses_minimal_config() {
    let cfg = YardConfig::from_path(&fixture("minimal.toml")).unwrap();
    assert_eq!(cfg.ros_distro, "jazzy");
}

#[test]
fn rejects_missing_ros_distro() {
    let err = YardConfig::from_path(&fixture("empty.toml")).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("ros_distro"),
        "expected error to mention `ros_distro`, got: {msg}"
    );
}

#[test]
fn rejects_unknown_top_level_key() {
    let err = YardConfig::from_path(&fixture("unknown_key.toml")).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unknown field") && msg.contains("flooble"),
        "expected unknown-field error mentioning `flooble`, got: {msg}"
    );
}

#[test]
fn rejects_invalid_toml_syntax() {
    let path = fixture("invalid_syntax.toml");
    let err = YardConfig::from_path(&path).unwrap_err();
    match &err {
        ConfigError::Parse { path: p, .. } => assert_eq!(p, &path),
        other => panic!("expected Parse error, got: {other:?}"),
    }
    // The Display impl embeds the offending path so users can find it.
    assert!(err.to_string().contains(path.to_str().unwrap()));
}

#[test]
fn missing_file_is_read_error() {
    let path = fixture("does-not-exist.toml");
    let err = YardConfig::from_path(&path).unwrap_err();
    match err {
        ConfigError::Read { path: p, .. } => assert_eq!(p, path),
        other => panic!("expected Read error, got: {other:?}"),
    }
}
