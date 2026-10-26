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
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use tempfile::tempdir;

/// How long a startup-failure run may take before the test calls it hung.
///
/// The success path here is an *exit*, so the daemon finishing its work is the
/// signal. A daemon that accepts its configuration instead keeps running, and
/// an unbounded wait would then block the whole suite rather than fail it —
/// which is exactly what a regression in token validation would cause.
const STARTUP_DEADLINE: Duration = Duration::from_secs(10);

/// Run `comenqd` with `--config` pointing at `config`, plus `extra` arguments.
fn run_daemon(config: &Path, extra: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_comenqd"));
    command
        .arg("--config")
        .arg(config)
        // Rebuild the child's environment from scratch rather than removing
        // known keys one by one. Any inherited `COMENQD_*` setting — including
        // ones added to `Config` after this test was written — merges over the
        // file and could make the child fail for a reason the fixture does not
        // describe. The binary links only system libraries, so clearing costs
        // nothing it needs.
        .env_clear()
        .env("COMENQD_SOCKET_PATH", "/tmp/comenqd-contract-test.sock")
        .env("COMENQD_QUEUE_PATH", "/tmp/comenqd-contract-test-queue");
    // `LLVM_PROFILE_FILE` is not configuration: `cargo llvm-cov` exports it to
    // place each instrumented process's profile, and a child that loses it
    // writes somewhere the collector is not reading. Forward only that.
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command.args(extra);
    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn comenqd");
    wait_with_deadline(child)
}

/// Wait for `child` to exit, failing the test if it outlives the deadline.
///
/// An overrunning child is killed and reaped before the panic so the failure
/// reports the child's own diagnostics and leaves no stray daemon behind.
fn wait_with_deadline(mut child: Child) -> Output {
    let deadline = Instant::now() + STARTUP_DEADLINE;
    loop {
        match child.try_wait().expect("poll comenqd") {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                child.kill().expect("kill overrunning comenqd");
                let output = child.wait_with_output().expect("reap overrunning comenqd");
                panic!(
                    "comenqd did not exit within {STARTUP_DEADLINE:?}; stdout was {:?}, stderr was {:?}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                );
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    child.wait_with_output().expect("collect comenqd output")
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
    // The fixture supplies no token, so a loader that silently treated an
    // absent path as empty configuration would fail here anyway, for the wrong
    // reason. Require the file-specific diagnostic.
    assert!(
        stderr.contains("Configuration file error in") && stderr.contains("not found"),
        "the failure must be the missing configuration file, not a later token \
         error; got {stderr:?}"
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
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.trim().is_empty(),
        "the failure must be explained on stderr"
    );
    // As above: treat malformed TOML falling back to an empty configuration as
    // a defect, not as an acceptable route to some later failure.
    assert!(
        stderr.contains("Configuration file error in") && stderr.contains("TOML parse error"),
        "the failure must be the TOML parse error, not a later token error; got {stderr:?}"
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
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.trim().is_empty(),
        "the failure must be explained on stderr"
    );
    // `Config::resolve_github_token` reports an absent credential through this
    // exact message. Without it, any other startup failure would satisfy the
    // assertions above and this test would no longer pin the contract it
    // exists to protect.
    assert!(
        stderr.contains("provide github_token, github_token_file, or github_token_files"),
        "the failure must name the missing credential, not merely fail; got {stderr:?}"
    );
}
