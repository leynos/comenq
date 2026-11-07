//! Configuration file loading, precedence, defaults, and CLI overrides.

use super::super::{
    CliArgs, Config, DEFAULT_COOLDOWN, DEFAULT_GITHUB_API_TIMEOUT_SECS,
    DEFAULT_RESTART_MIN_DELAY_MS,
};
use clap::Parser as _;
use ortho_config::OrthoError;
use rstest::rstest;
use std::fs;
use std::io;
use std::path::PathBuf;
use tempfile::tempdir;
use test_support::mock_env;

/// Return the `io::ErrorKind` behind an [`OrthoError::File`], if present.
///
/// `load_config_file` wraps every filesystem verdict in `OrthoError::File`
/// with an `io::Error` source, so the kind is what distinguishes "no such
/// file" from "not a regular file" without matching on a rendered message.
fn file_error_kind(error: &OrthoError) -> Option<io::ErrorKind> {
    let OrthoError::File { source, .. } = error else {
        return None;
    };
    source.downcast_ref::<io::Error>().map(io::Error::kind)
}

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

#[test]
fn apply_cli_overrides_preserves_unset_values_and_applies_set_values() {
    let mut cfg = Config {
        github_token: "file-token".into(),
        github_token_file: Some(PathBuf::from("/tmp/file-token")),
        github_token_files: Vec::new(),
        socket_path: PathBuf::from("/tmp/file.sock"),
        queue_path: PathBuf::from("/tmp/file-queue"),
        cooldown_period_seconds: 10,
        cooldown_flutter_seconds: 20,
        restart_min_delay_ms: 30,
        github_api_timeout_secs: 40,
    };

    Config::apply_cli_overrides(&mut cfg, &CliArgs::default());
    assert_eq!(cfg.github_token, "file-token");
    assert_eq!(
        cfg.github_token_file,
        Some(PathBuf::from("/tmp/file-token"))
    );
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/file.sock"));
    assert_eq!(cfg.queue_path, PathBuf::from("/tmp/file-queue"));
    assert_eq!(cfg.cooldown_period_seconds, 10);
    assert_eq!(cfg.github_api_timeout_secs, 40);
    assert_eq!(cfg.cooldown_flutter_seconds, 20);
    assert_eq!(cfg.restart_min_delay_ms, 30);

    let cli = CliArgs::try_parse_from([
        "comenqd",
        "--github-token",
        "cli-token",
        "--github-token-file",
        "/tmp/cli-token",
        "--socket-path",
        "/tmp/cli.sock",
        "--queue-path",
        "/tmp/cli-queue",
        "--cooldown-period-seconds",
        "60",
        "--github-api-timeout-secs",
        "70",
    ])
    .expect("parse CLI overrides");

    Config::apply_cli_overrides(&mut cfg, &cli);
    assert_eq!(cfg.github_token, "cli-token");
    assert_eq!(cfg.github_token_file, Some(PathBuf::from("/tmp/cli-token")));
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/cli.sock"));
    assert_eq!(cfg.queue_path, PathBuf::from("/tmp/cli-queue"));
    assert_eq!(cfg.cooldown_period_seconds, 60);
    assert_eq!(cfg.github_api_timeout_secs, 70);
    assert_eq!(cfg.cooldown_flutter_seconds, 20);
    assert_eq!(cfg.restart_min_delay_ms, 30);
}

#[rstest]
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
    let env = mock_env([("COMENQD_SOCKET_PATH", "/tmp/env.sock")]);
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

    let cfg = Config::from_file_with_cli(&cli.config, &cli, &env).expect("load config");

    assert_eq!(cfg.github_token, "cli-token");
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/cli.sock"));
    assert_eq!(cfg.queue_path, PathBuf::from("/tmp/cli-queue"));
    assert_eq!(cfg.cooldown_period_seconds, 30);
    assert_eq!(cfg.github_api_timeout_secs, 40);
}

#[rstest]
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

    let cfg = Config::from_file_with_cli(&cli.config, &cli, &mock_env([] as [(&str, &str); 0]))
        .expect("load config");

    assert_eq!(cfg.github_token, "cli-token");
    assert_eq!(cfg.github_token_file, Some(cli_token));
}

#[rstest]
fn loads_from_file() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        "github_token='abc'\nsocket_path='/tmp/s.sock'\nqueue_path='/tmp/q'",
    )
    .expect("write config fixture");
    let cfg =
        Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).expect("load config");
    assert_eq!(cfg.github_token, "abc");
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/s.sock"));
    assert_eq!(cfg.queue_path, PathBuf::from("/tmp/q"));
}

#[rstest]
fn error_when_missing_file() {
    let path = PathBuf::from("/nonexistent/file.toml");
    let res = Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0]));
    assert!(res.is_err());
}

#[rstest]
fn error_when_path_is_a_directory() {
    // ortho_config v0.3.0 returned `Ok(None)` for any path that was not a
    // file; v0.10.0 rejects a non-regular entry outright. Both are errors
    // once the missing-file `ok_or_else` runs, so `is_err()` alone would pass
    // under either behaviour and would not detect a regression. Assert the
    // `InvalidInput` kind that v0.10.0's directory rejection produces.
    let dir = tempdir().expect("create tempdir");

    let res = Config::from_file_with_env(dir.path(), &mock_env([] as [(&str, &str); 0]));

    let error = res.expect_err("a directory must not load as configuration");
    assert_eq!(
        file_error_kind(&error),
        Some(io::ErrorKind::InvalidInput),
        "v0.10.0 rejects a directory as invalid input, not as a missing file; got {error}",
    );
}

#[rstest]
#[case::invalid_toml("github_token='abc' this is not toml")]
#[case::missing_token("socket_path='/tmp/s.sock'")]
#[case::missing_token_file("github_token_file='/nonexistent/token'")]
fn invalid_configuration_errors(#[case] contents: &str) {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, contents).expect("write invalid config fixture");
    let res = Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0]));
    assert!(res.is_err());
}

#[rstest]
fn whitespace_only_inline_token_errors() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token=' \t '").expect("write whitespace token fixture");

    assert!(Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).is_err());
}

#[rstest]
fn defaults_are_applied() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'").expect("write config fixture");
    let cfg =
        Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).expect("load config");
    assert_eq!(
        cfg.socket_path,
        PathBuf::from(comenq_transport::DEFAULT_SOCKET_PATH)
    );
    assert_eq!(cfg.queue_path, PathBuf::from("/var/lib/comenq/queue"));
    assert_eq!(cfg.cooldown_period_seconds, DEFAULT_COOLDOWN);
    assert_eq!(cfg.cooldown_flutter_seconds, 0);
    assert_eq!(cfg.restart_min_delay_ms, DEFAULT_RESTART_MIN_DELAY_MS);
    assert_eq!(cfg.github_api_timeout_secs, DEFAULT_GITHUB_API_TIMEOUT_SECS);
}

#[rstest]
fn default_socket_path_uses_runtime_dir() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'").expect("write config fixture");
    let env = mock_env([("XDG_RUNTIME_DIR", "/run/user/1000")]);
    let cfg = Config::from_file_with_env(&path, &env).expect("load config");
    assert_eq!(
        cfg.socket_path,
        PathBuf::from("/run/user/1000/comenq/comenq.sock")
    );
}

/// Direct deserialization must not read `XDG_RUNTIME_DIR` from the process
/// environment; only the loader supplies the environment-aware default.
#[test]
fn direct_deserialization_uses_system_socket_path() {
    let cfg: Config = serde_json::from_value(serde_json::json!({ "github_token": "abc" }))
        .expect("deserialize config without socket_path");

    assert_eq!(
        cfg.socket_path,
        PathBuf::from(comenq_transport::DEFAULT_SOCKET_PATH)
    );
}

/// CLI arguments should take precedence over environment variables
/// and configuration file values when building the daemon `Config`.
#[rstest]
fn cli_overrides_env_and_file() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'\nsocket_path='/tmp/file.sock'")
        .expect("write config fixture");
    let env = mock_env([("COMENQD_SOCKET_PATH", "/tmp/env.sock")]);
    let cli = CliArgs::try_parse_from([
        "comenqd",
        "--config",
        path.to_str().expect("temp path is UTF-8"),
        "--socket-path",
        "/tmp/cli.sock",
    ])
    .expect("parse daemon CLI socket option");
    let cfg = Config::from_file_with_cli(&path, &cli, &env).expect("load config");
    assert_eq!(cfg.socket_path, PathBuf::from("/tmp/cli.sock"));
}

#[rstest]
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
    let cfg = Config::from_file_with_cli(&path, &cli, &mock_env([] as [(&str, &str); 0]))
        .expect("load config");
    assert_eq!(cfg.cooldown_period_seconds, 30);
}

#[rstest]
fn nonzero_flutter_loads_from_toml() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'\ncooldown_flutter_seconds=45")
        .expect("write config fixture");

    let cfg =
        Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).expect("load config");

    assert_eq!(cfg.cooldown_flutter_seconds, 45);
}

#[cfg(feature = "test-support")]
#[rstest]
#[case(|cfg: &test_support::daemon::TestConfig| Config::from(cfg))]
#[case(|cfg: &test_support::daemon::TestConfig| Config::from(cfg.clone()))]
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
