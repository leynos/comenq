//! Regression tests for the precedence tiers a command line can settle.
//!
//! `Config::from_file_with_cli_and_tokens` merges `COMENQD_*` variables over
//! the file unconditionally, and `apply_cli_overrides` runs after that. Every
//! test here therefore supplies a command-line value for the field it asserts,
//! so the final assignment overwrites every lower tier and a *well-formed*
//! inherited `COMENQD_*` variable cannot change the verdict. Measured: with
//! `COMENQD_SOCKET_PATH`, `COMENQD_QUEUE_PATH`, `COMENQD_COOLDOWN_PERIOD_SECONDS`,
//! `COMENQD_GITHUB_API_TIMEOUT_SECS`, `COMENQD_COOLDOWN_FLUTTER_SECONDS`,
//! `COMENQD_RESTART_MIN_DELAY_MS` and `XDG_RUNTIME_DIR` all exported, all five
//! tests still pass.
//!
//! One exposure remains, and it is older than this migration: a *malformed*
//! inherited variable makes `fig.extract()` fail before any override runs, so
//! no precedence tier is reached at all. That is a property of the loader, not
//! of these tests — under the same condition ten of the twenty baseline tests
//! in `config/tests/configuration.rs` fail identically — and closing it would
//! mean changing production behaviour rather than test structure.
//!
//! Every other tier — defaults, file, environment — is pinned in
//! `crates/comenqd/tests/config_environment.rs`, which runs the loader in a
//! child process whose environment is cleared and rebuilt, and is therefore
//! immune to both cases above. These in-process tests inject an empty
//! environment through [`test_support::mock_env`], so a stray `COMENQD_*`
//! variable in the developer's shell cannot influence them either.

use super::super::{CliArgs, Config};
use clap::Parser as _;
use rstest::rstest;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;
use test_support::mock_env;

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
/// Every row supplies a command-line value, because that is what makes the row
/// deterministic. `file_value` is always present too: the daemon's
/// configuration file is mandatory, so a row without one would not exercise a
/// reachable state. The literals differ between the two tiers, so reading the
/// wrong one shows up as a wrong value rather than an accidental match.
struct TierCase {
    file_value: &'static str,
    cli_value: &'static str,
    expected: &'static str,
}

#[rstest]
#[case::command_line_overrides_a_differing_file_value(TierCase {
    file_value: "/tmp/file.sock",
    cli_value: "/tmp/cli.sock",
    expected: "/tmp/cli.sock",
})]
#[case::command_line_value_equal_to_the_built_in_default_still_wins(TierCase {
    file_value: "/tmp/file.sock",
    cli_value: "/run/comenq/comenq.sock",
    expected: "/run/comenq/comenq.sock",
})]
fn socket_path_resolves_to_the_highest_present_tier(#[case] case: TierCase) {
    let dir = tempdir().expect("create tempdir");
    let path = write_config(
        &dir,
        &format!("{BASE_CONFIG}\nsocket_path='{}'", case.file_value),
    );
    let path_str = path.to_str().expect("temp path is UTF-8");
    let args = cli(&["--config", path_str, "--socket-path", case.cli_value]);

    let cfg = Config::from_file_with_cli(&args.config, &args, &mock_env([] as [(&str, &str); 0]))
        .expect("load config");

    assert_eq!(cfg.socket_path, PathBuf::from(case.expected));
}

#[rstest]
fn command_line_cooldown_equal_to_the_default_still_wins() {
    // The v0.10.0 §2 defect suppressed a command-line value that merely
    // restated the clap default, letting a lower tier win. This repository
    // applies overrides by hand and unconditionally, so the restated value
    // must win. `960` is `DEFAULT_COOLDOWN`; the file deliberately disagrees.
    let dir = tempdir().expect("create tempdir");
    let path = write_config(&dir, &format!("{BASE_CONFIG}\ncooldown_period_seconds=10"));
    let args = cli(&[
        "--config",
        path.to_str().expect("temp path is UTF-8"),
        "--cooldown-period-seconds",
        "960",
    ]);

    let cfg = Config::from_file_with_cli(&args.config, &args, &mock_env([] as [(&str, &str); 0]))
        .expect("load config");

    assert_eq!(cfg.cooldown_period_seconds, 960);
}

#[rstest]
fn command_line_scalar_values_parse_into_their_fields() {
    // Parsing of a command-line scalar into its field type, pinned separately
    // from the precedence claim above.
    let dir = tempdir().expect("create tempdir");
    let path = write_config(&dir, BASE_CONFIG);
    let args = cli(&[
        "--config",
        path.to_str().expect("temp path is UTF-8"),
        "--cooldown-period-seconds",
        "420",
        "--github-api-timeout-secs",
        "99",
    ]);

    let cfg = Config::from_file_with_cli(&args.config, &args, &mock_env([] as [(&str, &str); 0]))
        .expect("load config");

    assert_eq!(cfg.cooldown_period_seconds, 420);
    assert_eq!(cfg.github_api_timeout_secs, 99);
}

#[rstest]
fn command_line_comma_separated_token_file_is_not_split_into_a_pool() {
    // `--github-token-file` sets a single path and clears the pool, and a
    // comma is part of that path rather than a separator. The fixture file is
    // therefore really named with a comma, so a loader that split the argument
    // in two would look for paths that do not exist and fail loudly instead of
    // silently accepting the split.
    let dir = tempdir().expect("create tempdir");
    let path = write_config(&dir, BASE_CONFIG);
    let comma_named = dir.path().join("first.token,second.token");
    fs::write(&comma_named, "comma-token").expect("write comma-named token file");
    let args = cli(&[
        "--config",
        path.to_str().expect("temp path is UTF-8"),
        "--github-token-file",
        comma_named.to_str().expect("token path is UTF-8"),
    ]);

    let cfg = Config::from_file_with_cli(&args.config, &args, &mock_env([] as [(&str, &str); 0]))
        .expect("load config");

    assert_eq!(cfg.github_token_file, Some(comma_named));
    assert!(
        cfg.github_token_files.is_empty(),
        "an explicit single token file clears the pool"
    );
}
