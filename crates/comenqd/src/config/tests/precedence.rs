//! Regression tests for the in-process configuration precedence tiers.
//!
//! These pin the layered merge order — built-in defaults, then the TOML file,
//! then command-line overrides — for the tiers whose inputs this process can
//! supply without touching its own environment.
//!
//! The environment tier is deliberately *not* here. Figment's `Env` provider
//! reads `std::env::vars_os()` directly and offers no injection seam, so
//! exercising it in-process would mean mutating this test binary's
//! environment. Issue #237 §5 forbids adding that, so the environment cases
//! live in `crates/comenqd/tests/config_environment.rs`, which re-executes the
//! test binary in a child process with a cleared, rebuilt environment.

use super::super::{CliArgs, Config};
use clap::Parser as _;
use rstest::rstest;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

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
    cli_value: Option<&'static str>,
    expected: &'static str,
}

#[rstest]
#[case::default_when_no_layer_sets_a_value(TierCase {
    file_value: None,
    cli_value: None,
    expected: comenq_transport::DEFAULT_SOCKET_PATH,
})]
#[case::file_overrides_default(TierCase {
    file_value: Some("/tmp/file.sock"),
    cli_value: None,
    expected: "/tmp/file.sock",
})]
#[case::command_line_overrides_file(TierCase {
    file_value: Some("/tmp/file.sock"),
    cli_value: Some("/tmp/cli.sock"),
    expected: "/tmp/cli.sock",
})]
#[serial_test::serial]
fn socket_path_resolves_to_the_highest_present_tier(#[case] case: TierCase) {
    // Every present tier supplies a distinct literal, so reading the wrong
    // layer shows up as a wrong value rather than an accidental match.
    let dir = tempdir().expect("create tempdir");
    let contents = match case.file_value {
        Some(value) => format!("{BASE_CONFIG}\nsocket_path='{value}'"),
        None => BASE_CONFIG.to_string(),
    };
    let path = write_config(&dir, &contents);

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
    // value must win over the file.
    let dir = tempdir().expect("create tempdir");
    let path = write_config(&dir, &format!("{BASE_CONFIG}\ncooldown_period_seconds=10"));
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
fn file_scalar_values_parse_into_their_fields() {
    // The file layer still has to coerce TOML scalars into the field types;
    // the environment cases in `tests/config_environment.rs` cover the same
    // coercion for the layer that Figment reads from the surroundings.
    let dir = tempdir().expect("create tempdir");
    let path = write_config(
        &dir,
        &format!("{BASE_CONFIG}\ncooldown_period_seconds=420\ngithub_api_timeout_secs=99"),
    );

    let cfg = Config::from_file(&path).expect("load config");

    assert_eq!(cfg.cooldown_period_seconds, 420);
    assert_eq!(cfg.github_api_timeout_secs, 99);
}

#[rstest]
#[serial_test::serial]
fn comma_separated_token_files_are_not_split_into_a_pool() {
    // A quoted TOML string bearing a comma stays one string, so the `Vec`
    // field rejects it rather than splitting. This pins the file layer's
    // string-to-sequence rule; the equivalent rule for a value that arrives
    // through the environment provider is pinned in
    // `tests/config_environment.rs`, which is the only place a provider swap
    // could be observed.
    let dir = tempdir().expect("create tempdir");
    let joined = format!(
        "{},{}",
        dir.path().join("first.token").display(),
        dir.path().join("second.token").display()
    );
    let path = write_config(
        &dir,
        &format!("{BASE_CONFIG}\ngithub_token_files='{joined}'"),
    );

    let result = Config::from_file_with_cli(&path, &CliArgs::default());

    assert!(result.is_err(), "a comma is not a list separator here");
}
