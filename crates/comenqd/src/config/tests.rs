//! Tests for daemon configuration loading, overrides, and defaults.

use super::{
    CliArgs, Config, DEFAULT_COOLDOWN, DEFAULT_GITHUB_API_TIMEOUT_SECS,
    DEFAULT_RESTART_MIN_DELAY_MS,
};
use rstest::rstest;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

use test_support::EnvVarGuard;

#[rstest]
#[serial_test::serial]
fn loads_from_file() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        "github_token='abc'\nsocket_path='/tmp/s.sock'\nqueue_path='/tmp/q'",
    )
    .expect("write config fixture");
    let _guard = EnvVarGuard::remove("COMENQD_SOCKET_PATH");
    let cfg = Config::from_file(&path).expect("load config");
    assert_eq!(cfg.github_token, "abc");
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/s.sock"));
    assert_eq!(cfg.queue_path, PathBuf::from("/tmp/q"));
}

#[rstest]
#[serial_test::serial]
fn error_when_missing_file() {
    let path = PathBuf::from("/nonexistent/file.toml");
    let res = Config::from_file(&path);
    assert!(res.is_err());
}

#[rstest]
#[serial_test::serial]
fn env_vars_override_file() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'\nsocket_path='/tmp/s.sock'")
        .expect("write config fixture");
    let _guard = EnvVarGuard::set("COMENQD_SOCKET_PATH", "/tmp/override.sock");
    let cfg = Config::from_file(&path).expect("load config");
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/override.sock"));
}

#[rstest]
#[serial_test::serial]
fn error_with_invalid_toml() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc' this is not toml").expect("write invalid toml");
    let res = Config::from_file(&path);
    assert!(res.is_err());
}

#[rstest]
#[serial_test::serial]
fn error_when_missing_token() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "socket_path='/tmp/s.sock'").expect("write config without token");
    let res = Config::from_file(&path);
    assert!(res.is_err());
}

#[rstest]
#[serial_test::serial]
fn defaults_are_applied() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'").expect("write config fixture");
    let _socket_guard = EnvVarGuard::remove("COMENQD_SOCKET_PATH");
    let _xdg_guard = EnvVarGuard::remove("XDG_RUNTIME_DIR");
    let cfg = Config::from_file(&path).expect("load config");
    assert_eq!(
        cfg.socket_path,
        PathBuf::from(comenq_lib::DEFAULT_SOCKET_PATH)
    );
    assert_eq!(cfg.queue_path, PathBuf::from("/var/lib/comenq/queue"));
    assert_eq!(cfg.cooldown_period_seconds, DEFAULT_COOLDOWN);
    assert_eq!(cfg.cooldown_flutter_seconds, 0);
    assert_eq!(cfg.restart_min_delay_ms, DEFAULT_RESTART_MIN_DELAY_MS);
    assert_eq!(cfg.github_api_timeout_secs, DEFAULT_GITHUB_API_TIMEOUT_SECS);
}

#[rstest]
#[serial_test::serial]
fn default_socket_path_uses_runtime_dir() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'").expect("write config fixture");
    let _socket_guard = EnvVarGuard::remove("COMENQD_SOCKET_PATH");
    let _xdg_guard = EnvVarGuard::set("XDG_RUNTIME_DIR", "/run/user/1000");
    let cfg = Config::from_file(&path).expect("load config");
    assert_eq!(
        cfg.socket_path,
        PathBuf::from("/run/user/1000/comenq/comenq.sock")
    );
}

/// CLI arguments should take precedence over environment variables
/// and configuration file values when building the daemon `Config`.
#[rstest]
#[serial_test::serial]
fn cli_overrides_env_and_file() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'\nsocket_path='/tmp/file.sock'")
        .expect("write config fixture");
    let _guard = EnvVarGuard::set("COMENQD_SOCKET_PATH", "/tmp/env.sock");
    let cli = CliArgs {
        config: path.clone(),
        github_token: None,
        github_token_file: None,
        socket_path: Some(PathBuf::from("/tmp/cli.sock")),
        queue_path: None,
        cooldown_period_seconds: None,
        github_api_timeout_secs: None,
    };
    let cfg = Config::from_file_with_cli(&path, &cli).expect("load config");
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/cli.sock"));
}

#[rstest]
#[serial_test::serial]
fn cli_overrides_cooldown() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'\ncooldown_period_seconds=10")
        .expect("write config fixture");
    let cli = CliArgs {
        config: path.clone(),
        github_token: None,
        github_token_file: None,
        socket_path: None,
        queue_path: None,
        cooldown_period_seconds: Some(30),
        github_api_timeout_secs: None,
    };
    let cfg = Config::from_file_with_cli(&path, &cli).expect("load config");
    assert_eq!(cfg.cooldown_period_seconds, 30);
}

#[rstest]
#[serial_test::serial]
fn token_file_overrides_inline_token() {
    let dir = tempdir().expect("create tempdir");
    let token_path = dir.path().join("token");
    fs::write(&token_path, "s3cret\n").expect("write token file");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!(
            "github_token='inline'\ngithub_token_file='{}'",
            token_path.display()
        ),
    )
    .expect("write config fixture");
    let cfg = Config::from_file(&path).expect("load config");
    assert_eq!(cfg.github_token, "s3cret");
}

#[rstest]
#[serial_test::serial]
fn token_file_alone_suffices() {
    let dir = tempdir().expect("create tempdir");
    let token_path = dir.path().join("token");
    fs::write(&token_path, "s3cret").expect("write token file");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!("github_token_file='{}'", token_path.display()),
    )
    .expect("write config fixture");
    let cfg = Config::from_file(&path).expect("load config");
    assert_eq!(cfg.github_token, "s3cret");
}

#[rstest]
#[serial_test::serial]
fn missing_token_file_errors() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token_file='/nonexistent/token'").expect("write config fixture");
    assert!(Config::from_file(&path).is_err());
}

#[rstest]
#[serial_test::serial]
fn empty_token_file_errors() {
    let dir = tempdir().expect("create tempdir");
    let token_path = dir.path().join("token");
    fs::write(&token_path, "\n").expect("write empty token file");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!("github_token_file='{}'", token_path.display()),
    )
    .expect("write config fixture");
    assert!(Config::from_file(&path).is_err());
}

#[rstest]
#[serial_test::serial]
fn token_file_expands_credentials_directory() {
    let dir = tempdir().expect("create tempdir");
    let token_path = dir.path().join("token");
    fs::write(&token_path, "cred-token").expect("write token file");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token_file='${CREDENTIALS_DIRECTORY}/token'")
        .expect("write config fixture");
    let _guard = EnvVarGuard::set(
        "CREDENTIALS_DIRECTORY",
        dir.path().to_str().expect("tempdir path is UTF-8"),
    );
    let cfg = Config::from_file(&path).expect("load config");
    assert_eq!(cfg.github_token, "cred-token");
}

#[rstest]
#[serial_test::serial]
fn token_file_with_unset_placeholder_errors() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        "github_token_file='${COMENQD_TEST_UNSET_VARIABLE}/token'",
    )
    .expect("write config fixture");
    let _guard = EnvVarGuard::remove("COMENQD_TEST_UNSET_VARIABLE");
    assert!(Config::from_file(&path).is_err());
}

#[cfg(feature = "test-support")]
#[rstest]
#[case(|cfg: &test_support::daemon::TestConfig| Config::from(cfg))]
#[case(|cfg: &test_support::daemon::TestConfig| Config::from(cfg.clone()))]
#[serial_test::serial]
fn converts_from_test_config(#[case] conv: fn(&test_support::daemon::TestConfig) -> Config) {
    use test_support::temp_config;

    let tmp = tempdir().expect("create tempdir");
    let test_cfg = temp_config(&tmp).with_cooldown(42);
    let cfg = conv(&test_cfg);

    assert_eq!(cfg.github_token, test_cfg.github_token);
    assert_eq!(cfg.socket_path, test_cfg.socket_path);
    assert_eq!(cfg.queue_path, test_cfg.queue_path);
    assert_eq!(
        cfg.cooldown_period_seconds,
        test_cfg.cooldown_period_seconds
    );
    assert_eq!(cfg.restart_min_delay_ms, test_cfg.restart_min_delay_ms);
    assert_eq!(
        cfg.github_api_timeout_secs,
        test_cfg.github_api_timeout_secs
    );
}

#[rstest]
#[serial_test::serial]
fn token_files_form_the_rotation_pool() {
    let dir = tempdir().expect("create tempdir");
    let first = dir.path().join("pandalump-token");
    let second = dir.path().join("buzzybee-token");
    fs::write(&first, "alpha-token\n").expect("write first token");
    fs::write(&second, "bravo-token\n").expect("write second token");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!(
            "github_token_files=['{}','{}']",
            first.display(),
            second.display()
        ),
    )
    .expect("write config fixture");
    let cfg = Config::from_file(&path).expect("load config");
    let tokens = cfg.github_tokens().expect("resolve tokens");
    let summary: Vec<(&str, &str)> = tokens
        .iter()
        .map(|named| (named.name.as_str(), named.token.as_str()))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("pandalump-token", "alpha-token"),
            ("buzzybee-token", "bravo-token"),
        ]
    );
}

#[rstest]
#[serial_test::serial]
fn token_files_suffice_without_an_inline_token() {
    let dir = tempdir().expect("create tempdir");
    let file = dir.path().join("only-token");
    fs::write(&file, "solo\n").expect("write token");
    let path = dir.path().join("config.toml");
    fs::write(&path, format!("github_token_files=['{}']", file.display()))
        .expect("write config fixture");
    let cfg = Config::from_file(&path).expect("token files alone should satisfy validation");
    assert!(cfg.github_token.is_empty());
}

#[rstest]
#[serial_test::serial]
fn empty_rotation_token_file_errors() {
    let dir = tempdir().expect("create tempdir");
    let file = dir.path().join("empty-token");
    fs::write(&file, "  \n").expect("write blank token");
    let path = dir.path().join("config.toml");
    fs::write(&path, format!("github_token_files=['{}']", file.display()))
        .expect("write config fixture");
    assert!(Config::from_file(&path).is_err());
}

#[rstest]
#[serial_test::serial]
fn missing_rotation_token_file_errors() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!(
            "github_token_files=['{}']",
            dir.path().join("nope-token").display()
        ),
    )
    .expect("write config fixture");
    assert!(Config::from_file(&path).is_err());
}

#[rstest]
#[serial_test::serial]
fn single_inline_token_forms_a_default_pool() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'").expect("write config fixture");
    let cfg = Config::from_file(&path).expect("load config");
    let tokens = cfg.github_tokens().expect("resolve tokens");
    assert_eq!(tokens.len(), 1);
    let named = tokens.first().expect("single token");
    assert_eq!(named.name, "default");
    assert_eq!(named.token, "abc");
}
