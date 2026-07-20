//! Client-side communication with the `comenqd` daemon.
//!
//! This module serializes a protocol request, sends it to the daemon over
//! its Unix Domain Socket, and renders the reply. It is separated from
//! `lib.rs` so that argument parsing remains focused and the network logic
//! is easily testable.

use comenq_lib::protocol::{Request, Response};
use std::path::PathBuf;
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};
use tracing::warn;

use crate::output::{render_entry, render_history, render_put};
use crate::{Args, Command};

/// Errors that can occur when interacting with the daemon.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Connecting to the daemon failed.
    #[error("failed to connect to daemon: {0}")]
    Connect(#[from] std::io::Error),
    /// Serializing the request or parsing the reply failed.
    #[error("failed to encode or decode a daemon message: {0}")]
    Serialize(#[from] serde_json::Error),
    /// Writing the request to the socket failed.
    #[error("failed to write to daemon: {0}")]
    Write(#[source] std::io::Error),
    /// Shutting down the socket failed.
    #[error("failed to close connection: {0}")]
    Shutdown(#[source] std::io::Error),
    /// Reading the daemon's reply failed.
    #[error("failed to read daemon reply: {0}")]
    Read(#[source] std::io::Error),
    /// The daemon reported a failure.
    #[error("daemon refused the request: {0}")]
    Daemon(String),
    /// The daemon's reply did not match the request.
    #[error("unexpected reply from daemon")]
    UnexpectedResponse,
}

/// Connect to the first candidate socket that accepts a connection.
///
/// A daemon that exits without unlinking its socket leaves a stale file
/// behind; connecting to it fails (typically `ECONNREFUSED`), and the next
/// candidate is tried, so a stale user socket never shadows a healthy
/// system daemon. The last connection error is returned when every
/// candidate fails.
async fn connect_first(candidates: &[PathBuf]) -> Result<UnixStream, ClientError> {
    let mut last_error: Option<std::io::Error> = None;
    for candidate in candidates {
        match UnixStream::connect(candidate).await {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                warn!(socket = %candidate.display(), error = %e, "socket candidate refused");
                last_error = Some(e);
            }
        }
    }
    Err(ClientError::Connect(last_error.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no socket candidates to try")
    })))
}

/// Send `request` to the first reachable candidate and parse the reply.
async fn transact(candidates: &[PathBuf], request: &Request) -> Result<Response, ClientError> {
    let payload = serde_json::to_vec(request)?;
    let mut stream = connect_first(candidates).await?;
    stream
        .write_all(&payload)
        .await
        .map_err(ClientError::Write)?;
    stream.shutdown().await.map_err(ClientError::Shutdown)?;
    let mut reply = Vec::new();
    stream
        .read_to_end(&mut reply)
        .await
        .map_err(ClientError::Read)?;
    Ok(serde_json::from_slice(&reply)?)
}

/// Execute the parsed command against the daemon and print the outcome.
///
/// # Examples
///
/// ```no_run
/// # use comenq::{Args, Command, run};
/// # use std::path::PathBuf;
/// # async fn try_run() -> Result<(), comenq::ClientError> {
/// let args = Args {
///     socket: Some(PathBuf::from("/tmp/comenq.sock")),
///     command: Command::List,
/// };
/// run(args).await?;
/// # Ok(())
/// # }
/// ```
pub async fn run(args: Args) -> Result<(), ClientError> {
    let request = args.command.to_request();
    let response = transact(&args.socket_candidates(), &request).await?;
    let (entry, entries, history) = match response {
        Response::Error { message } => return Err(ClientError::Daemon(message)),
        Response::Ok {
            entry,
            entries,
            history,
        } => (entry, entries, history),
    };
    match &args.command {
        Command::Put { .. } => {
            let entry = entry.ok_or(ClientError::UnexpectedResponse)?;
            println!("{}", render_put(&entry));
        }
        Command::List => {
            let entries = entries.ok_or(ClientError::UnexpectedResponse)?;
            if entries.is_empty() {
                println!("No comments queued.");
            } else {
                for entry in &entries {
                    println!("{}", render_entry(entry));
                }
            }
        }
        Command::Bump { id } => println!("Moved {id} to the head of the queue."),
        Command::Bust { id } => println!("Moved {id} to the tail of the queue."),
        Command::Del { id } => println!("Removed {id} from the queue."),
        Command::Hist { .. } => {
            let history = history.ok_or(ClientError::UnexpectedResponse)?;
            if history.is_empty() {
                println!("No posting history recorded.");
            } else {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                for record in &history {
                    println!("{}", render_history(record, now));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Round-trip tests for the client transport.
    use super::{ClientError, run};
    use crate::{Args, Command};
    use comenq_lib::protocol::{PendingEntry, Request, Response};
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    fn put_args(socket: std::path::PathBuf) -> Args {
        Args {
            socket: Some(socket),
            command: Command::Put {
                repo_slug: "octocat/hello-world".parse().expect("slug"),
                pr_number: 1,
                comment_body: "Hi".into(),
                now: false,
            },
        }
    }

    /// Accept one connection, capture the request, and reply.
    fn spawn_daemon(listener: UnixListener, reply: Response) -> tokio::task::JoinHandle<Request> {
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.expect("read");
            let request = serde_json::from_slice::<Request>(&buf).expect("deserialize");
            let bytes = serde_json::to_vec(&reply).expect("serialize reply");
            stream.write_all(&bytes).await.expect("write reply");
            request
        })
    }

    #[tokio::test]
    async fn run_sends_put_request_and_accepts_reply() {
        let dir = tempdir().expect("temp dir");
        let socket = dir.path().join("sock");
        let listener = UnixListener::bind(&socket).expect("bind socket");
        let reply = Response::entry(PendingEntry {
            id: "1a2b3c4d".into(),
            eta_seconds: 0,
            owner: "octocat".into(),
            repo: "hello-world".into(),
            pr_number: 1,
            body: "Hi".into(),
        });
        let accept = spawn_daemon(listener, reply);

        run(put_args(socket)).await.expect("run succeeds");
        let request = accept.await.expect("join");
        let Request::Put { request, immediate } = request else {
            panic!("expected put request, got {request:?}");
        };
        assert_eq!(request.owner, "octocat");
        assert_eq!(request.repo, "hello-world");
        assert_eq!(request.pr_number, 1);
        assert_eq!(request.body, "Hi");
        assert!(!immediate, "put must default to deferred posting");
    }

    #[tokio::test]
    async fn run_sends_hist_request_and_accepts_reply() {
        let dir = tempdir().expect("temp dir");
        let socket = dir.path().join("sock");
        let listener = UnixListener::bind(&socket).expect("bind socket");
        let reply = Response::history(vec![comenq_lib::protocol::HistoryEntry {
            id: "1a2b3c4d".into(),
            posted_at: 1_000,
            success: true,
            error: None,
            owner: "octocat".into(),
            repo: "hello-world".into(),
            pr_number: 1,
            body: "Hi".into(),
        }]);
        let accept = spawn_daemon(listener, reply);

        let args = Args {
            socket: Some(socket),
            command: Command::Hist { limit: Some(2) },
        };
        run(args).await.expect("run succeeds");
        let request = accept.await.expect("join");
        assert_eq!(request, Request::Hist { limit: Some(2) });
    }

    #[tokio::test]
    async fn run_surfaces_daemon_errors() {
        let dir = tempdir().expect("temp dir");
        let socket = dir.path().join("sock");
        let listener = UnixListener::bind(&socket).expect("bind socket");
        let accept = spawn_daemon(listener, Response::error("queue unavailable"));

        let err = run(put_args(socket)).await.expect_err("should error");
        assert!(matches!(err, ClientError::Daemon(m) if m == "queue unavailable"));
        accept.await.expect("join");
    }

    #[tokio::test]
    async fn run_rejects_mismatched_reply() {
        let dir = tempdir().expect("temp dir");
        let socket = dir.path().join("sock");
        let listener = UnixListener::bind(&socket).expect("bind socket");
        // A bare Ok reply lacks the entry a put expects.
        let accept = spawn_daemon(listener, Response::ok());

        let err = run(put_args(socket)).await.expect_err("should error");
        assert!(matches!(err, ClientError::UnexpectedResponse));
        accept.await.expect("join");
    }

    /// A stale socket file (bound once, listener dropped, file left behind)
    /// must not shadow a live daemon later in the candidate list.
    #[tokio::test]
    async fn connect_first_skips_stale_sockets() {
        let dir = tempdir().expect("temp dir");
        let stale = dir.path().join("stale.sock");
        drop(UnixListener::bind(&stale).expect("bind stale socket"));
        assert!(stale.exists(), "stale socket file should remain on disk");

        let live = dir.path().join("live.sock");
        let listener = UnixListener::bind(&live).expect("bind live socket");

        let stream = super::connect_first(&[stale.clone(), live.clone()])
            .await
            .expect("should fall back to the live socket");
        drop(stream);
        drop(listener);
    }

    /// Every candidate failing yields the connection error.
    #[tokio::test]
    async fn connect_first_reports_failure_when_all_candidates_fail() {
        let dir = tempdir().expect("temp dir");
        let stale = dir.path().join("stale.sock");
        drop(UnixListener::bind(&stale).expect("bind stale socket"));
        let missing = dir.path().join("missing.sock");

        let err = super::connect_first(&[stale, missing])
            .await
            .expect_err("all candidates should fail");
        assert!(matches!(err, ClientError::Connect(_)));
    }

    #[tokio::test]
    async fn run_errors_when_socket_missing() {
        let dir = tempdir().expect("temp dir");
        let socket = dir.path().join("nosock");

        let err = run(put_args(socket)).await.expect_err("should error");
        assert!(matches!(err, ClientError::Connect(_)));
    }
}
