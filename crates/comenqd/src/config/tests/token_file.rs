//! Tests for the legacy single-token file credential path.

use super::super::{CliArgs, Config, MAX_GITHUB_TOKEN_FILE_BYTES};
use clap::Parser as _;
use rstest::rstest;
use std::fs;
use tempfile::tempdir;
use test_support::mock_env;

#[rstest]
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

    let cfg = Config::from_file_with_cli(&path, &cli, &mock_env([] as [(&str, &str); 0]))
        .expect("load config");

    assert_eq!(cfg.github_token, "cli-token");
}

#[rstest]
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
    let cfg =
        Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).expect("load config");
    assert_eq!(cfg.github_token, "s3cret");
}

#[rstest]
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
    let cfg =
        Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).expect("load config");
    assert_eq!(cfg.github_token, "s3cret");
}

#[rstest]
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
    assert!(Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).is_err());
}

#[rstest]
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

    assert!(Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).is_err());
}

#[rstest]
fn token_file_expands_credentials_directory() {
    let dir = tempdir().expect("create tempdir");
    let token_path = dir.path().join("token");
    fs::write(&token_path, "cred-token").expect("write token file");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token_file='${CREDENTIALS_DIRECTORY}/token'")
        .expect("write config fixture");
    let env = mock_env([(
        "CREDENTIALS_DIRECTORY",
        dir.path().to_str().expect("tempdir path is UTF-8"),
    )]);
    let cfg = Config::from_file_with_env(&path, &env).expect("load config");
    assert_eq!(cfg.github_token, "cred-token");
}

#[rstest]
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
    assert!(Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).is_err());
}

#[rstest]
fn missing_token_file_with_inline_token_errors() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        "github_token='inline-token'\ngithub_token_file='/nonexistent/token'",
    )
    .expect("write config fixture");

    assert!(Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).is_err());
}
