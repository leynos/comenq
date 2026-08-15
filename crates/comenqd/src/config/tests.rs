//! Tests for daemon configuration loading, overrides, and defaults.

use super::{
    CliArgs, Config, DEFAULT_CLIENT_CHANNEL_CAPACITY, DEFAULT_COOLDOWN,
    DEFAULT_GITHUB_API_TIMEOUT_SECS, DEFAULT_RESTART_MIN_DELAY_MS, MAX_GITHUB_TOKEN_FILE_BYTES,
};
use clap::Parser as _;
use rstest::rstest;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

use test_support::EnvVarGuard;

#[test]
fn github_token_file_cli_option_parses() {
    let args = CliArgs::try_parse_from([
        "comenqd",
        "--config",
        "/tmp/config.toml",
        "--github-token-file",
        "/run/credentials/comenqd/token",
    ])
    .expect("parse daemon CLI options");

    assert_eq!(
        args.github_token_file,
        Some(PathBuf::from("/run/credentials/comenqd/token"))
    );
}

#[rstest]
#[serial_test::serial]
fn parsed_cli_options_override_environment_and_file() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        concat!(
            "github_token='file-token'\n",
            "socket_path='/tmp/file.sock'\n",
            "queue_path='/tmp/file-queue'\n",
            "cooldown_period_seconds=10\n",
            "github_api_timeout_secs=20\n",
        ),
    )
    .expect("write config fixture");
    let _guard = EnvVarGuard::set("COMENQD_SOCKET_PATH", "/tmp/env.sock");
    let cli = CliArgs::try_parse_from([
        "comenqd",
        "--config",
        path.to_str().expect("temp path is UTF-8"),
        "--github-token",
        "cli-token",
        "--socket-path",
        "/tmp/cli.sock",
        "--queue-path",
        "/tmp/cli-queue",
        "--cooldown-period-seconds",
        "30",
        "--github-api-timeout-secs",
        "40",
    ])
    .expect("parse daemon CLI options");

    let cfg = Config::from_file_with_cli(&cli.config, &cli).expect("load config");

    assert_eq!(cfg.github_token, "cli-token");
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/cli.sock"));
    assert_eq!(cfg.queue_path, PathBuf::from("/tmp/cli-queue"));
    assert_eq!(cfg.cooldown_period_seconds, 30);
    assert_eq!(cfg.github_api_timeout_secs, 40);
}

#[rstest]
#[serial_test::serial]
fn parsed_cli_token_file_overrides_toml_token_file() {
    let dir = tempdir().expect("create tempdir");
    let toml_token = dir.path().join("toml-token");
    let cli_token = dir.path().join("cli-token");
    fs::write(&toml_token, "toml-token").expect("write TOML token file");
    fs::write(&cli_token, "cli-token").expect("write CLI token file");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!(
            "github_token='inline-token'\ngithub_token_file='{}'",
            toml_token.display()
        ),
    )
    .expect("write config fixture");
    let cli = CliArgs::try_parse_from([
        "comenqd",
        "--config",
        path.to_str().expect("temp path is UTF-8"),
        "--github-token-file",
        cli_token.to_str().expect("temp path is UTF-8"),
    ])
    .expect("parse daemon CLI token file option");

    let cfg = Config::from_file_with_cli(&cli.config, &cli).expect("load config");

    assert_eq!(cfg.github_token, "cli-token");
    assert_eq!(cfg.github_token_file, Some(cli_token));
}

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
#[case::invalid_toml("github_token='abc' this is not toml")]
#[case::missing_token("socket_path='/tmp/s.sock'")]
#[case::missing_token_file("github_token_file='/nonexistent/token'")]
#[serial_test::serial]
fn invalid_configuration_errors(#[case] contents: &str) {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, contents).expect("write invalid config fixture");
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
    assert_eq!(cfg.client_channel_capacity, DEFAULT_CLIENT_CHANNEL_CAPACITY);
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
    let cli = CliArgs::try_parse_from([
        "comenqd",
        "--config",
        path.to_str().expect("temp path is UTF-8"),
        "--socket-path",
        "/tmp/cli.sock",
    ])
    .expect("parse daemon CLI socket option");
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
    let cli = CliArgs::try_parse_from([
        "comenqd",
        "--config",
        path.to_str().expect("temp path is UTF-8"),
        "--cooldown-period-seconds",
        "30",
    ])
    .expect("parse daemon CLI cooldown option");
    let cfg = Config::from_file_with_cli(&path, &cli).expect("load config");
    assert_eq!(cfg.cooldown_period_seconds, 30);
}

#[rstest]
#[serial_test::serial]
fn cli_token_overrides_token_file() {
    let dir = tempdir().expect("create tempdir");
    let token_path = dir.path().join("token");
    fs::write(&token_path, "file-token").expect("write token file");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!("github_token_file='{}'", token_path.display()),
    )
    .expect("write config fixture");
    let cli = CliArgs::try_parse_from([
        "comenqd",
        "--config",
        path.to_str().expect("temp path is UTF-8"),
        "--github-token",
        "cli-token",
        "--github-token-file",
        token_path.to_str().expect("temp path is UTF-8"),
    ])
    .expect("parse daemon CLI token options");

    let cfg = Config::from_file_with_cli(&path, &cli).expect("load config");

    assert_eq!(cfg.github_token, "cli-token");
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
fn empty_token_file_errors() {
    let dir = tempdir().expect("create tempdir");
    let token_path = dir.path().join("token");
    fs::write(&token_path, "\n").expect("write empty token file");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!(
            "github_token='inline-token'\ngithub_token_file='{}'",
            token_path.display()
        ),
    )
    .expect("write config fixture");
    assert!(Config::from_file(&path).is_err());
}

#[rstest]
#[serial_test::serial]
fn oversized_token_file_errors() {
    let dir = tempdir().expect("create tempdir");
    let token_path = dir.path().join("token");
    fs::write(&token_path, "x".repeat(MAX_GITHUB_TOKEN_FILE_BYTES + 1))
        .expect("write oversized token file");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!(
            "github_token='inline-token'\ngithub_token_file='{}'",
            token_path.display()
        ),
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
        concat!(
            "github_token='inline-token'\n",
            "github_token_file='${COMENQD_TEST_UNSET_VARIABLE}/token'",
        ),
    )
    .expect("write config fixture");
    let _guard = EnvVarGuard::remove("COMENQD_TEST_UNSET_VARIABLE");
    assert!(Config::from_file(&path).is_err());
}

#[rstest]
#[serial_test::serial]
fn missing_token_file_with_inline_token_errors() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        "github_token='inline-token'\ngithub_token_file='/nonexistent/token'",
    )
    .expect("write config fixture");

    assert!(Config::from_file(&path).is_err());
}

#[rstest]
#[serial_test::serial]
fn nonzero_flutter_loads_from_toml() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'\ncooldown_flutter_seconds=45")
        .expect("write config fixture");

    let cfg = Config::from_file(&path).expect("load config");

    assert_eq!(cfg.cooldown_flutter_seconds, 45);
}

#[rstest]
#[serial_test::serial]
fn nonzero_flutter_loads_from_environment() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'").expect("write config fixture");
    let _guard = EnvVarGuard::set("COMENQD_COOLDOWN_FLUTTER_SECONDS", "45");

    let cfg = Config::from_file(&path).expect("load config");

    assert_eq!(cfg.cooldown_flutter_seconds, 45);
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
    assert_eq!(
        cfg.client_channel_capacity,
        test_cfg.client_channel_capacity
    );
}
