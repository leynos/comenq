//! Process-level checks on the daemon's startup failure contract.
//!
//! `Config::load_with_tokens` returning `Err` is only half the contract: an
//! operator sees the exit status and the streams of the binary. These tests
//! run the real executable so the `color_eyre` reporting path, the exit code,
//! and the choice of standard error rather than standard output are all
//! exercised as a user would observe them.

#![cfg(feature = "test-support")]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::tempdir;

/// Run `comenqd` with `--config` pointing at `config`, plus `extra` arguments.
fn run_daemon(config: &Path, extra: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_comenqd"));
    command
        .arg("--config")
        .arg(config)
        // Isolate the child from any developer environment so the run is
        // deterministic and cannot reach a real socket or queue.
        .env_remove("COMENQD_SOCKET_PATH")
        .env_remove("COMENQD_QUEUE_PATH")
        .env_remove("COMENQD_GITHUB_TOKEN")
        .env_remove("COMENQD_GITHUB_TOKEN_FILE")
        .env_remove("COMENQD_GITHUB_TOKEN_FILES")
        .env_remove("COMENQD_COOLDOWN_PERIOD_SECONDS")
        .env_remove("COMENQD_GITHUB_API_TIMEOUT_SECS")
        .env("COMENQD_SOCKET_PATH", "/tmp/comenqd-contract-test.sock")
        .env("COMENQD_QUEUE_PATH", "/tmp/comenqd-contract-test-queue");
    command.args(extra);
    command.output().expect("run comenqd")
}

#[test]
fn missing_configuration_exits_non_zero_and_reports_on_stderr() {
    let dir = tempdir().expect("create tempdir");
    let missing = dir.path().join("absent.toml");

    let output = run_daemon(&missing, &[]);

    assert!(
        !output.status.success(),
        "a missing configuration must fail; stdout was {:?}",
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(
        output.stdout.is_empty(),
        "diagnostics belong on stderr, not stdout; got {:?}",
        String::from_utf8_lossy(&output.stdout),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.trim().is_empty(),
        "the failure must be explained on stderr"
    );
}

#[test]
fn malformed_configuration_exits_non_zero_and_reports_on_stderr() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc' this is not toml").expect("write bad fixture");

    let output = run_daemon(&path, &[]);

    assert!(
        !output.status.success(),
        "malformed TOML must fail; stdout was {:?}",
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(
        output.stdout.is_empty(),
        "diagnostics belong on stderr, not stdout; got {:?}",
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "the failure must be explained on stderr"
    );
}

#[test]
fn missing_token_exits_non_zero_and_reports_on_stderr() {
    let dir = tempdir().expect("create tempdir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "socket_path='/tmp/comenqd-contract-test.sock'")
        .expect("write token-less fixture");

    let output = run_daemon(&path, &[]);

    assert!(
        !output.status.success(),
        "a token-less configuration must fail; stdout was {:?}",
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(
        output.stdout.is_empty(),
        "diagnostics belong on stderr, not stdout; got {:?}",
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "the failure must be explained on stderr"
    );
}
