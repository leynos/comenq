//! Environment-tier configuration tests, run in isolated child processes.
//!
//! Figment's `Env` provider reads `std::env::vars_os()` directly and exposes no
//! injection seam, so the `COMENQD_*` merge layer can only be exercised
//! through a process's real environment. Issue #237 §5 forbids introducing new
//! process-environment mutation, and names the sanctioned alternative:
//! isolated child processes with controlled environments.
//!
//! Re-executing this test binary is the cheapest way to get such a child. It
//! shares the parent's `--test-threads=1` and `--nocapture` arguments, so a
//! bare re-exec would run every test again and recurse. The child therefore
//! returns immediately unless `COMENQD_ENV_TIER_CHILD` is set, which only
//! [`spawn_child`] sets.

#![cfg(feature = "test-support")]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tempfile::tempdir;

/// Variable marking the re-executed child, so it runs the child body rather
/// than another full test pass.
const CHILD_MARKER: &str = "COMENQD_ENV_TIER_CHILD";

/// Configuration file the child loads.
const CHILD_CONFIG: &str = "COMENQD_ENV_TIER_CONFIG";

/// File the child writes its serialized configuration to.
///
/// The result cannot travel on standard output: libtest writes its own
/// `running 1 test` banner there, which would sit in front of the JSON.
const CHILD_RESULT: &str = "COMENQD_ENV_TIER_RESULT";

/// How long the child may take before the test calls it hung.
const CHILD_DEADLINE: Duration = Duration::from_secs(30);

/// The `Config` fields these tests inspect.
///
/// A dedicated projection keeps the assertions focused on the values under
/// test, rather than restating the whole configuration surface.
#[derive(Debug, Deserialize, Serialize)]
struct Resolved {
    socket_path: PathBuf,
    queue_path: PathBuf,
    cooldown_period_seconds: u64,
    github_api_timeout_secs: u64,
    github_token_files: Vec<PathBuf>,
}

impl Resolved {
    /// Project the fields under test out of a loaded daemon configuration.
    fn project(config: comenqd::config::Config) -> Self {
        Self {
            socket_path: config.socket_path,
            queue_path: config.queue_path,
            cooldown_period_seconds: config.cooldown_period_seconds,
            github_api_timeout_secs: config.github_api_timeout_secs,
            github_token_files: config.github_token_files,
        }
    }
}

/// Name of the child body, as the test filter sees it.
///
/// The child is told to run exactly this test. Reusing the parent's arguments
/// instead would make the child run the *caller's* test, which spawns a child
/// of its own and recurses until the deadline trips.
const CHILD_BODY: &str = "child_loads_configuration_with_the_supplied_environment";

/// Start a child that re-executes this binary with `config` and `vars`.
///
/// The child's environment is rebuilt from scratch, so the only `COMENQD_*`
/// variables in it are the ones a case sets. That total isolation is the
/// point: a developer's own `COMENQD_*` settings cannot bend a verdict.
///
/// `LLVM_PROFILE_FILE` is the one exception, and it is not a configuration
/// variable. `cargo llvm-cov` exports it to tell every instrumented process
/// where its profile belongs; a child that does not receive it writes to the
/// default location instead, which silently drops the child's coverage and
/// litters the working directory. Forwarding it keeps the collector's output
/// whole without letting anything else through.
fn spawn_child(config: &Path, result: &Path, vars: &[(&str, &OsStr)]) -> Child {
    let mut command = Command::new(std::env::current_exe().expect("locate test binary"));
    command
        .args(["--exact", CHILD_BODY, "--test-threads=1", "--nocapture"])
        .env_clear()
        .env(CHILD_MARKER, "1")
        .env(CHILD_CONFIG, config)
        .env(CHILD_RESULT, result);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    for (key, value) in vars {
        command.env(key, value);
    }
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn environment-tier child")
}

/// Wait for `child` to exit, failing the test if it outlives the deadline.
///
/// An overrunning child is killed and reaped before the panic, so a hung child
/// reports its own diagnostics and leaves nothing behind.
fn wait_with_deadline(mut child: Child, what: &str) -> Output {
    let deadline = Instant::now() + CHILD_DEADLINE;
    loop {
        match child.try_wait().expect("poll environment-tier child") {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                child
                    .kill()
                    .expect("kill overrunning environment-tier child");
                let output = child
                    .wait_with_output()
                    .expect("reap overrunning environment-tier child");
                panic!(
                    "{what} did not exit within {CHILD_DEADLINE:?}; stdout was {:?}, stderr was {:?}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                );
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    child.wait_with_output().expect("collect child output")
}

/// Run the child body with `config` and `vars` as its whole environment.
fn load_in_child(config: &Path, vars: &[(&str, &OsStr)], what: &str) -> Resolved {
    let dir = tempdir().expect("create result tempdir");
    let result = dir.path().join("resolved.json");
    let output = wait_with_deadline(spawn_child(config, &result, vars), what);
    assert!(
        output.status.success(),
        "{what} failed: stdout was {:?}, stderr was {:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let serialized = std::fs::read(&result)
        .unwrap_or_else(|error| panic!("{what} wrote no configuration to {result:?}: {error}"));
    serde_json::from_slice(&serialized)
        .unwrap_or_else(|error| panic!("{what} did not serialize a configuration: {error}"))
}

/// Write `contents` as the configuration file for an environment case.
fn write_config(dir: &tempfile::TempDir, contents: &str) -> PathBuf {
    let path = dir.path().join("config.toml");
    std::fs::write(&path, contents).expect("write config fixture");
    path
}

/// The child body: load the configuration and print the fields under test.
///
/// Returns immediately in an ordinary run, so re-executing the whole binary
/// costs one no-op test per child rather than a second full suite.
#[test]
fn child_loads_configuration_with_the_supplied_environment() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    let config = std::env::var_os(CHILD_CONFIG).expect("the parent supplies a configuration path");
    let result = std::env::var_os(CHILD_RESULT).expect("the parent supplies a result path");
    let loaded = comenqd::config::Config::from_file(Path::new(&config))
        .unwrap_or_else(|error| panic!("loading {config:?} in the child failed: {error}"));
    let serialized =
        serde_json::to_vec(&Resolved::project(loaded)).expect("serialize resolved configuration");
    std::fs::write(&result, serialized).expect("write resolved configuration");
}

/// With no value in any layer, the built-in default wins.
#[test]
fn defaults_apply_when_no_layer_sets_a_value() {
    let dir = tempdir().expect("create tempdir");
    let config = write_config(&dir, "github_token='file-token'");

    let resolved = load_in_child(&config, &[], "default case");

    assert_eq!(
        resolved.socket_path,
        PathBuf::from(comenq_transport::DEFAULT_SOCKET_PATH)
    );
}

/// The file layer overrides the default.
#[test]
fn file_overrides_default() {
    let dir = tempdir().expect("create tempdir");
    let config = write_config(
        &dir,
        "github_token='file-token'\nsocket_path='/tmp/file.sock'",
    );

    let resolved = load_in_child(&config, &[], "file case");

    assert_eq!(resolved.socket_path, PathBuf::from("/tmp/file.sock"));
}

/// The environment overrides a differing file value.
#[test]
fn environment_overrides_file() {
    let dir = tempdir().expect("create tempdir");
    let config = write_config(
        &dir,
        "github_token='file-token'\nsocket_path='/tmp/file.sock'",
    );
    let socket = PathBuf::from("/tmp/env.sock");

    let resolved = load_in_child(
        &config,
        &[("COMENQD_SOCKET_PATH", socket.as_os_str())],
        "environment case",
    );

    assert_eq!(resolved.socket_path, socket);
}

/// The environment overrides a field the file leaves unset.
///
/// This is the pairing that distinguishes "the environment layer ran" from
/// "the environment layer was read and then lost to the default", which a case
/// with a file value present cannot see.
#[test]
fn environment_overrides_an_absent_file_value() {
    let dir = tempdir().expect("create tempdir");
    let config = write_config(&dir, "github_token='file-token'");
    let queue = PathBuf::from("/tmp/env-queue");

    let resolved = load_in_child(
        &config,
        &[("COMENQD_QUEUE_PATH", queue.as_os_str())],
        "environment-over-default case",
    );

    assert_eq!(resolved.queue_path, queue);
}

/// An integer-valued environment variable is parsed, not left as a string.
///
/// The literal differs from `DEFAULT_COOLDOWN`, so ignoring the environment
/// layer would fail here rather than accidentally pass.
#[test]
fn environment_integer_is_parsed() {
    let dir = tempdir().expect("create tempdir");
    let config = write_config(&dir, "github_token='file-token'");

    let resolved = load_in_child(
        &config,
        &[("COMENQD_COOLDOWN_PERIOD_SECONDS", OsStr::new("420"))],
        "environment integer case",
    );

    assert_eq!(resolved.cooldown_period_seconds, 420);
    // The untouched neighbour keeps a wrong-field parse visible.
    assert_eq!(resolved.github_api_timeout_secs, 30);
}

/// A comma-separated value is not split into a token pool.
///
/// Figment's `Env` leaves a comma-bearing value as one string, so the `Vec`
/// field rejects it. `ortho_config::CsvEnv` enables comma splitting by default
/// and would accept this as two paths, turning a startup error into a working
/// two-token pool. This case pins the rejection for the provider the daemon
/// actually installs.
#[test]
fn comma_separated_token_files_are_not_split() {
    let dir = tempdir().expect("create tempdir");
    let config = write_config(&dir, "github_token='file-token'");
    let files = format!(
        "{},{}",
        dir.path().join("first.token").display(),
        dir.path().join("second.token").display()
    );

    let result = dir.path().join("resolved.json");
    let output = wait_with_deadline(
        spawn_child(
            &config,
            &result,
            &[("COMENQD_GITHUB_TOKEN_FILES", OsStr::new(&files))],
        ),
        "comma case",
    );

    // The child panics on a load error by design, so a non-zero status is the
    // expected outcome. The assertion names the merge failure so an unrelated
    // panic cannot satisfy it.
    assert!(
        !output.status.success(),
        "a comma is not a list separator here; stdout was {:?}",
        String::from_utf8_lossy(&output.stdout),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("loading") && stderr.contains("Failed to gather configuration"),
        "the failure must come from parsing the comma-bearing value; got {stderr:?}",
    );
}

/// An array literal is the supported spelling for a sequence.
#[test]
fn array_literal_populates_the_pool() {
    let dir = tempdir().expect("create tempdir");
    let config = write_config(&dir, "github_token='file-token'");
    let first = dir.path().join("first.token");
    let second = dir.path().join("second.token");
    std::fs::write(&first, "first-token").expect("write first token file");
    std::fs::write(&second, "second-token").expect("write second token file");
    let literal = format!("[\"{}\",\"{}\"]", first.display(), second.display());

    let resolved = load_in_child(
        &config,
        &[("COMENQD_GITHUB_TOKEN_FILES", OsStr::new(&literal))],
        "array literal case",
    );

    assert_eq!(resolved.github_token_files, vec![first, second]);
}
