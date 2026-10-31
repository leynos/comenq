//! Tests for ordered token-pool resolution and startup ownership.

use super::super::{CliArgs, Config, MAX_GITHUB_TOKEN_FILE_BYTES};
use clap::Parser as _;
use rstest::rstest;
use std::fs;
use tempfile::tempdir;
use test_support::mock_env;

#[rstest]
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
    let cfg =
        Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).expect("load config");
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
fn token_pool_rejects_duplicate_trimmed_values_without_disclosing_tokens() {
    let dir = tempdir().expect("create tempdir");
    let first = dir.path().join("first-token");
    let second = dir.path().join("second-token");
    fs::write(&first, "shared-secret\n").expect("write first token");
    fs::write(&second, "  shared-secret \n").expect("write second token");

    let error = super::super::tokens::resolve_pool(
        &[first.clone(), second.clone()],
        "inline-token",
        &mock_env([] as [(&str, &str); 0]),
    )
    .expect_err("duplicate trimmed tokens must be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains(&first.display().to_string()));
    assert!(error.to_string().contains(&second.display().to_string()));
    assert!(!error.to_string().contains("shared-secret"));
}

#[rstest]
fn token_pool_takes_precedence_over_single_token_sources() {
    let dir = tempdir().expect("create tempdir");
    let pool_token = dir.path().join("pool-token");
    fs::write(&pool_token, "pool-token-value").expect("write pool token");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!(
            "github_token='inline-token'\ngithub_token_file='/missing/single-token'\ngithub_token_files=['{}']",
            pool_token.display()
        ),
    )
    .expect("write config fixture");

    let (cfg, tokens) = Config::from_file_with_cli_and_tokens(
        &path,
        &CliArgs::default(),
        &mock_env([] as [(&str, &str); 0]),
    )
    .expect("pool should override single-token sources");

    assert_eq!(cfg.github_token, "inline-token");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].token, "pool-token-value");
}

#[rstest]
fn cli_token_overrides_an_invalid_configured_pool() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token_files=['/missing/pool-token']").expect("write config fixture");
    let cli = CliArgs::try_parse_from([
        "comenqd",
        "--config",
        path.to_str().expect("config path is UTF-8"),
        "--github-token",
        "cli-token",
    ])
    .expect("parse CLI token");

    let (cfg, tokens) =
        Config::from_file_with_cli_and_tokens(&path, &cli, &mock_env([] as [(&str, &str); 0]))
            .expect("CLI token should clear the pool");

    assert!(cfg.github_token_files.is_empty());
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].name, "default");
    assert_eq!(tokens[0].token, "cli-token");
}

#[rstest]
fn cli_token_file_overrides_a_configured_pool() {
    let dir = tempdir().expect("create tempdir");
    let cli_token = dir.path().join("cli-token");
    fs::write(&cli_token, "cli-file-token").expect("write CLI token file");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token_files=['/missing/pool-token']").expect("write config fixture");
    let cli = CliArgs::try_parse_from([
        "comenqd",
        "--config",
        path.to_str().expect("config path is UTF-8"),
        "--github-token-file",
        cli_token.to_str().expect("token path is UTF-8"),
    ])
    .expect("parse CLI token-file override");

    let (cfg, tokens) =
        Config::from_file_with_cli_and_tokens(&path, &cli, &mock_env([] as [(&str, &str); 0]))
            .expect("CLI token file should clear the pool");

    assert!(cfg.github_token_files.is_empty());
    assert_eq!(cfg.github_token, "cli-file-token");
    assert_eq!(tokens[0].token, "cli-file-token");
}

#[rstest]
fn token_pool_expands_a_leading_environment_placeholder() {
    let dir = tempdir().expect("create tempdir");
    let token_path = dir.path().join("pool-token");
    fs::write(&token_path, "expanded-token").expect("write pool token");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        "github_token_files=['${COMENQD_TOKEN_POOL_DIR}/pool-token']",
    )
    .expect("write config fixture");
    let env = mock_env([(
        "COMENQD_TOKEN_POOL_DIR",
        dir.path().to_str().expect("tempdir path is UTF-8"),
    )]);

    let (_, tokens) = Config::from_file_with_cli_and_tokens(&path, &CliArgs::default(), &env)
        .expect("resolve token pool path");

    assert_eq!(tokens[0].token, "expanded-token");
}

#[rstest]
fn token_pool_rejects_an_unset_environment_placeholder() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        "github_token_files=['${COMENQD_MISSING_TOKEN_POOL_DIR}/pool-token']",
    )
    .expect("write config fixture");

    assert!(Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).is_err());
}

#[rstest]
fn token_pool_rejects_an_unterminated_environment_placeholder() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        "github_token_files=['${COMENQD_TOKEN_POOL_DIR/pool-token']",
    )
    .expect("write config fixture");

    let error = Config::from_file_with_cli_and_tokens(
        &path,
        &CliArgs::default(),
        &mock_env([] as [(&str, &str); 0]),
    )
    .expect_err("an unterminated placeholder must fail config resolution");

    assert!(
        error
            .to_string()
            .contains("unterminated environment placeholder")
    );
}

#[rstest]
fn token_pool_rejects_an_oversized_file() {
    let dir = tempdir().expect("create tempdir");
    let token_path = dir.path().join("pool-token");
    fs::write(&token_path, "x".repeat(MAX_GITHUB_TOKEN_FILE_BYTES + 1))
        .expect("write oversized token file");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!("github_token_files=['{}']", token_path.display()),
    )
    .expect("write config fixture");

    assert!(Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).is_err());
}

#[rstest]
fn loaded_token_pool_is_owned_and_does_not_follow_later_file_changes() {
    let dir = tempdir().expect("create tempdir");
    let token_path = dir.path().join("pool-token");
    fs::write(&token_path, "startup-token").expect("write pool token");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!("github_token_files=['{}']", token_path.display()),
    )
    .expect("write config fixture");

    let (config, tokens) = Config::from_file_with_cli_and_tokens(
        &path,
        &CliArgs::default(),
        &mock_env([] as [(&str, &str); 0]),
    )
    .expect("load config and token pool together");
    fs::write(&token_path, "changed-token").expect("replace pool token after loading");

    assert_eq!(tokens[0].token, "startup-token");
    assert_eq!(
        config.github_tokens().expect("resolve current pool")[0].token,
        "changed-token"
    );
}

#[rstest]
fn token_files_suffice_without_an_inline_token() {
    let dir = tempdir().expect("create tempdir");
    let file = dir.path().join("only-token");
    fs::write(&file, "solo\n").expect("write token");
    let path = dir.path().join("config.toml");
    fs::write(&path, format!("github_token_files=['{}']", file.display()))
        .expect("write config fixture");
    let cfg = Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0]))
        .expect("token files alone should satisfy validation");
    assert!(cfg.github_token.is_empty());
}

#[rstest]
fn empty_rotation_token_file_errors() {
    let dir = tempdir().expect("create tempdir");
    let file = dir.path().join("empty-token");
    fs::write(&file, "  \n").expect("write blank token");
    let path = dir.path().join("config.toml");
    fs::write(&path, format!("github_token_files=['{}']", file.display()))
        .expect("write config fixture");
    assert!(Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).is_err());
}

#[rstest]
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
    assert!(Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).is_err());
}

#[rstest]
fn single_inline_token_forms_a_default_pool() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc'").expect("write config fixture");
    let cfg =
        Config::from_file_with_env(&path, &mock_env([] as [(&str, &str); 0])).expect("load config");
    let tokens = cfg.github_tokens().expect("resolve tokens");
    assert_eq!(tokens.len(), 1);
    let named = tokens.first().expect("single token");
    assert_eq!(named.name, "default");
    assert_eq!(named.token, "abc");
}
