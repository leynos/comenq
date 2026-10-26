//! Regression tests for each configuration precedence tier.
//!
//! These pin the layered merge order — built-in defaults, then the TOML file,
//! then the `COMENQD_*` environment, then command-line overrides — and the
//! scalar parsing the environment layer owes to Figment's `Env` provider.
//! They belong beside the migration to `ortho_config` v0.10.0 because that
//! release re-exports a `CsvEnv` provider with different parsing, and a
//! careless swap would silently change what these tiers mean.

use super::super::{CliArgs, Config};
use clap::Parser as _;
use rstest::rstest;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;
use test_support::EnvVarGuard;

/// Base configuration used by the tier tests.
const BASE_CONFIG: &str = "github_token='file-token'";

/// Write `contents` to `config.toml` inside `dir`.
fn write_config(dir: &tempfile::TempDir, contents: &str) -> PathBuf {
    let path = dir.path().join("config.toml");
    fs::write(&path, contents).expect("write config fixture");
    path
}

/// Parse `args` as command-line overrides.
fn cli(args: &[&str]) -> CliArgs {
    CliArgs::try_parse_from(std::iter::once("comenqd").chain(args.iter().copied()))
        .expect("parse daemon CLI options")
}

/// One row of the precedence truth table.
///
/// Each layer is optional so a row can express "this layer is absent" as
/// opposed to "this layer holds a particular value". `expected` is the only
/// `PathBuf` that is not optional: every row must resolve to something.
struct TierCase {
    file_value: Option<&'static str>,
    env_value: Option<&'static str>,
    cli_value: Option<&'static str>,
    expected: &'static str,
}

#[rstest]
#[case::default_when_no_layer_sets_a_value(TierCase {
    file_value: None,
    env_value: None,
    cli_value: None,
    expected: comenq_transport::DEFAULT_SOCKET_PATH,
})]
#[case::file_overrides_default(TierCase {
    file_value: Some("/tmp/file.sock"),
    env_value: None,
    cli_value: None,
    expected: "/tmp/file.sock",
})]
#[case::environment_overrides_file(TierCase {
    file_value: Some("/tmp/file.sock"),
    env_value: Some("/tmp/env.sock"),
    cli_value: None,
    expected: "/tmp/env.sock",
})]
#[case::command_line_overrides_environment_and_file(TierCase {
    file_value: Some("/tmp/file.sock"),
    env_value: Some("/tmp/env.sock"),
    cli_value: Some("/tmp/cli.sock"),
    expected: "/tmp/cli.sock",
})]
#[serial_test::serial]
fn socket_path_resolves_to_the_highest_present_tier(#[case] case: TierCase) {
    // Every row supplies a distinct literal per present tier, so reading the
    // wrong layer shows up as a wrong value rather than an accidental match.
    let dir = tempdir().expect("create tempdir");
    let contents = match case.file_value {
        Some(value) => format!("{BASE_CONFIG}\nsocket_path='{value}'"),
        None => BASE_CONFIG.to_string(),
    };
    let path = write_config(&dir, &contents);

    // Always clear both socket-related variables first so a row that omits
    // `env_value` really has no environment layer.
    let _socket_guard = match case.env_value {
        Some(value) => EnvVarGuard::set("COMENQD_SOCKET_PATH", value),
        None => EnvVarGuard::remove("COMENQD_SOCKET_PATH"),
    };
    let _xdg_guard = EnvVarGuard::remove("XDG_RUNTIME_DIR");

    let cfg = match case.cli_value {
        Some(value) => {
            let path_str = path.to_str().expect("temp path is UTF-8");
            let args = cli(&["--config", path_str, "--socket-path", value]);
            Config::from_file_with_cli(&args.config, &args)
        }
        None => Config::from_file(&path),
    }
    .expect("load config");

    assert_eq!(cfg.socket_path, PathBuf::from(case.expected));
}

#[rstest]
#[serial_test::serial]
fn command_line_value_equal_to_the_default_still_wins() {
    // The old derive defect masked a restated clap default, letting a lower
    // tier win. This repository applies overrides by hand, so the restated
    // value must win over the environment and the file.
    let dir = tempdir().expect("create tempdir");
    let path = write_config(&dir, &format!("{BASE_CONFIG}\ncooldown_period_seconds=10"));
    let _guard = EnvVarGuard::set("COMENQD_COOLDOWN_PERIOD_SECONDS", "20");
    let args = cli(&[
        "--config",
        path.to_str().expect("temp path is UTF-8"),
        "--cooldown-period-seconds",
        "960",
    ]);

    let cfg = Config::from_file_with_cli(&args.config, &args).expect("load config");

    assert_eq!(cfg.cooldown_period_seconds, 960);
}

#[rstest]
#[serial_test::serial]
fn environment_cooldown_parses_as_an_integer() {
    // The value must differ from `DEFAULT_COOLDOWN` (960), otherwise the
    // assertion would hold even if the environment layer were ignored and the
    // default simply won.
    let dir = tempdir().expect("create tempdir");
    let path = write_config(&dir, BASE_CONFIG);
    let _guard = EnvVarGuard::set("COMENQD_COOLDOWN_PERIOD_SECONDS", "420");

    let cfg = Config::from_file(&path).expect("load config");

    assert_eq!(cfg.cooldown_period_seconds, 420);
}

#[rstest]
#[serial_test::serial]
fn environment_wins_over_a_differing_file_value() {
    // A distinct literal per tier makes a wrong tier choice observable: the
    // file and the environment disagree, so reading either one is decisive.
    let dir = tempdir().expect("create tempdir");
    let path = write_config(&dir, &format!("{BASE_CONFIG}\ngithub_api_timeout_secs=10"));
    let _guard = EnvVarGuard::set("COMENQD_GITHUB_API_TIMEOUT_SECS", "99");

    let cfg = Config::from_file(&path).expect("load config");

    assert_eq!(cfg.github_api_timeout_secs, 99);
}

#[rstest]
#[serial_test::serial]
fn environment_comma_separated_value_is_not_split_into_a_pool() {
    // Figment's `Env` leaves a comma-bearing value as one string, so a `Vec`
    // field rejects it rather than splitting. `ortho_config::CsvEnv` enables
    // comma splitting by default and would accept this as two paths, so an
    // environment-provider swap would turn a startup error into a working
    // two-token pool. Pinning the rejection is what makes that swap visible.
    let dir = tempdir().expect("create tempdir");
    let joined = format!(
        "{},{}",
        dir.path().join("first.token").display(),
        dir.path().join("second.token").display()
    );
    let path = write_config(&dir, BASE_CONFIG);
    let _guard = EnvVarGuard::set("COMENQD_GITHUB_TOKEN_FILES", &joined);

    let result = Config::from_file_with_cli(&path, &CliArgs::default());

    assert!(result.is_err(), "a comma is not a list separator here");
}

#[rstest]
#[serial_test::serial]
fn environment_field_array_literal_populates_the_pool() {
    // The supported spelling for a sequence in the environment layer is an
    // array literal, which Figment parses before falling back to a string.
    let dir = tempdir().expect("create tempdir");
    let first = dir.path().join("first.token");
    let second = dir.path().join("second.token");
    fs::write(&first, "first-token").expect("write first token file");
    fs::write(&second, "second-token").expect("write second token file");
    let literal = format!("[\"{}\",\"{}\"]", first.display(), second.display());
    let path = write_config(&dir, BASE_CONFIG);
    let _guard = EnvVarGuard::set("COMENQD_GITHUB_TOKEN_FILES", &literal);

    let cfg = Config::from_file_with_cli(&path, &CliArgs::default()).expect("load config");

    assert_eq!(cfg.github_token_files, vec![first, second]);
}
