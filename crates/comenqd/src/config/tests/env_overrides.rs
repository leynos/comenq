//! `COMENQD_*` environment overrides and the guard that keeps them complete.
//!
//! These tests inject an in-memory environment, so they neither read nor
//! mutate the process environment.

use super::super::Config;
use super::super::env_layer::ENV_OVERRIDE_FIELDS;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;
use test_support::mock_env;

/// Guard against adding a `Config` field without a corresponding override.
#[test]
fn environment_override_fields_cover_every_config_field() {
    let config = Config {
        github_token: String::new(),
        github_token_file: None,
        github_token_files: Vec::new(),
        socket_path: PathBuf::from("/tmp/socket"),
        queue_path: PathBuf::from("/tmp/queue"),
        cooldown_period_seconds: 0,
        cooldown_flutter_seconds: 0,
        restart_min_delay_ms: 0,
        github_api_timeout_secs: 0,
    };
    let serialized = serde_json::to_value(&config).expect("serialize config");
    let mut actual: Vec<&str> = serialized
        .as_object()
        .expect("config serializes to an object")
        .keys()
        .map(String::as_str)
        .collect();
    actual.sort_unstable();
    let mut expected = ENV_OVERRIDE_FIELDS.to_vec();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[test]
fn env_vars_override_file() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'\nsocket_path='/tmp/s.sock'")
        .expect("write config fixture");
    let env = mock_env([("COMENQD_SOCKET_PATH", "/tmp/override.sock")]);
    let cfg = Config::from_file_with_env(&path, &env).expect("load config");
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/override.sock"));
}

/// Figment's previous provider matched names through `UncasedStr`, so a
/// lower-case spelling was honoured and must stay honoured after the
/// dependency-injection rewrite.
#[test]
fn differently_cased_env_vars_are_read() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'\nsocket_path='/tmp/s.sock'")
        .expect("write config fixture");
    let env = mock_env([("comenqd_socket_path", "/tmp/override.sock")]);
    let cfg = Config::from_file_with_env(&path, &env).expect("load config");
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/override.sock"));
}

/// Hash iteration order must not decide which casing wins.
#[test]
fn upper_case_env_var_wins_when_several_casings_exist() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'").expect("write config fixture");
    let env = mock_env([
        ("comenqd_socket_path", "/tmp/lower.sock"),
        ("COMENQD_SOCKET_PATH", "/tmp/upper.sock"),
    ]);
    let cfg = Config::from_file_with_env(&path, &env).expect("load config");
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/upper.sock"));
}

#[test]
fn nonzero_flutter_loads_from_environment() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'").expect("write config fixture");
    let env = mock_env([("COMENQD_COOLDOWN_FLUTTER_SECONDS", "45")]);

    let cfg = Config::from_file_with_env(&path, &env).expect("load config");

    assert_eq!(cfg.cooldown_flutter_seconds, 45);
}
