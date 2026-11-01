//! Production environment entry points with a non-Unicode socket override.
//!
//! `Args::socket_candidates` and the `run` path behind it read `COMENQ_SOCKET`
//! through `std::env::var_os` so a socket path holding raw bytes survives.
//! Unit tests inject a provider and therefore never see the real environment,
//! which leaves the production accessor unexercised; issue #237 §5 forbids
//! mutating this process's environment to close that gap. These tests instead
//! give a child process the variable: the first re-executes this binary, and
//! the second runs the real `comenq` executable.
//!
//! Re-executing this binary would run every test again, so the child returns
//! immediately unless `COMENQ_PRODUCTION_ENV_CHILD` is set. It is told to run
//! exactly one test by name, because inheriting the parent's filter would make
//! it run the caller's test and recurse.

use std::error::Error;
use std::ffi::OsStr;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::process::{Child, Output, Stdio};
use std::time::{Duration, Instant};

use clap::Parser;
use comenq::Args;
use comenq_lib::protocol::{Request, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::process::Command;

/// Variable marking the re-executed child.
const CHILD_MARKER: &str = "COMENQ_PRODUCTION_ENV_CHILD";

/// File the child writes its candidate paths to.
const CHILD_RESULT: &str = "COMENQ_PRODUCTION_ENV_RESULT";

/// How long a child or an accept may take before the test calls it hung.
const DEADLINE: Duration = Duration::from_secs(30);

/// The child body name, as the test filter sees it.
const CANDIDATES_CHILD_BODY: &str = "child_reports_socket_candidates";

/// A socket path whose file name holds bytes that are not valid UTF-8.
///
/// The bytes are chosen so that any lossy conversion produces a different
/// name, which a lossy round trip back to a path would not match.
fn non_unicode_socket(directory: &tempfile::TempDir) -> PathBuf {
    directory
        .path()
        .join(OsStr::from_bytes(b"comenq-\xff\xfe.sock"))
}

/// Write `paths` as raw bytes, separated by NUL.
///
/// The result cannot travel on standard output, where libtest writes its own
/// banner. Serializing text would be simpler but a path under test is precisely
/// one a text format cannot represent.
fn encode_paths(paths: &[PathBuf]) -> Vec<u8> {
    let mut encoded = Vec::new();
    for path in paths {
        encoded.extend_from_slice(path.as_os_str().as_bytes());
        encoded.push(0);
    }
    encoded
}

/// Decode the format written by [`encode_paths`].
fn decode_paths(mut encoded: &[u8]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    while let Some(end) = encoded.iter().position(|byte| *byte == 0) {
        paths.push(PathBuf::from(std::ffi::OsString::from_vec(
            encoded[..end].to_vec(),
        )));
        encoded = &encoded[end + 1..];
    }
    paths
}

/// Start a child that re-executes this binary with `vars` in its environment.
fn spawn_child(vars: &[(&str, &OsStr)], result: &std::path::Path) -> Child {
    std::process::Command::new(std::env::current_exe().expect("locate test binary"))
        .args([
            "--exact",
            CANDIDATES_CHILD_BODY,
            "--test-threads=1",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .env(CHILD_RESULT, result)
        .envs(vars.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn production-environment child")
}

/// Wait for `child` to exit, failing the test if it outlives [`DEADLINE`].
fn wait_with_deadline(mut child: Child) -> Output {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match child.try_wait().expect("poll production-environment child") {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                child.kill().expect("kill overrunning child");
                let output = child.wait_with_output().expect("reap overrunning child");
                panic!(
                    "child did not exit within {DEADLINE:?}; stdout was {:?}, stderr was {:?}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                );
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    child.wait_with_output().expect("collect child output")
}

/// The child body: report the socket candidates the real environment yields.
#[test]
fn child_reports_socket_candidates() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    let result = std::env::var_os(CHILD_RESULT).expect("the parent supplies a result path");
    let args = Args::try_parse_from(["comenq", "list"]).expect("arguments parse");
    std::fs::write(&result, encode_paths(&args.socket_candidates()))
        .expect("write socket candidates");
}

/// `Args::socket_candidates` keeps a non-Unicode `COMENQ_SOCKET`.
#[test]
fn socket_candidates_keep_a_non_unicode_override() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let socket = non_unicode_socket(&directory);
    let result = directory.path().join("candidates.bin");
    let output = wait_with_deadline(spawn_child(
        &[("COMENQ_SOCKET", socket.as_os_str())],
        &result,
    ));
    assert!(
        output.status.success(),
        "the child failed: stdout was {:?}, stderr was {:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let decoded = decode_paths(&std::fs::read(&result)?);
    assert_eq!(decoded, vec![socket]);
    Ok(())
}

/// The `comenq` binary connects to a non-Unicode `COMENQ_SOCKET`.
///
/// Reaching the socket proves the whole production path — argument parsing,
/// candidate resolution, and the client's connect — treated the override as an
/// OS string. A UTF-8-only read would have reported it as unset and probed the
/// discovery paths instead, leaving this listener untouched.
#[tokio::test]
async fn binary_connects_to_a_non_unicode_socket_override() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let socket = non_unicode_socket(&directory);
    let listener = UnixListener::bind(&socket)?;
    let child = Command::new(env!("CARGO_BIN_EXE_comenq"))
        .arg("list")
        .env("COMENQ_SOCKET", socket.as_os_str())
        .env("RUST_LOG", "off")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    // Accepting is the assertion: an incorrect candidate list would never
    // connect here, and the timeout turns that into a failure rather than a
    // hang.
    let (mut stream, _) = tokio::time::timeout(DEADLINE, listener.accept())
        .await
        .map_err(|_| "the client never connected to the non-Unicode socket")??;
    let mut payload = Vec::new();
    stream.read_to_end(&mut payload).await?;
    let request: Request = serde_json::from_slice(&payload)?;
    assert_eq!(request, Request::List);
    stream
        .write_all(&serde_json::to_vec(&Response::entries(Vec::new()))?)
        .await?;
    stream.shutdown().await?;

    let output = tokio::time::timeout(DEADLINE, child.wait_with_output())
        .await
        .map_err(|_| "the client did not exit")??;
    assert!(
        output.status.success(),
        "the client failed: stdout was {:?}, stderr was {:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    Ok(())
}
