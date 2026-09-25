//! Client-side communication with the `comenqd` daemon.
//!
//! This module serializes a protocol request, sends it to the daemon over
//! its Unix Domain Socket, and renders the reply. It is separated from
//! `lib.rs` so that argument parsing remains focused and the network logic
//! is easily testable.

use comenq_lib::protocol::{MAX_RESPONSE_BYTES, Request, Response};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};
use tracing::{debug, warn};

use crate::output::{render_entry, render_put};
use crate::{Args, Command};

/// Maximum time to wait for a complete daemon reply.
const CLIENT_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// The daemon reply exceeds the protocol size limit.
    #[error("daemon reply exceeds {MAX_RESPONSE_BYTES} bytes")]
    ReplyTooLarge,
    /// The daemon did not finish its reply before the client deadline.
    #[error("timed out waiting for daemon reply")]
    ReplyTimeout,
    /// The daemon reported a failure.
    #[error("daemon refused the request: {0}")]
    Daemon(String),
    /// The daemon's reply did not match the request.
    #[error("unexpected reply from daemon")]
    UnexpectedResponse,
    /// Writing the command result to standard output failed.
    #[error("failed to write command output: {0}")]
    Output(#[source] std::io::Error),
}

/// Connect to the first candidate socket that accepts a connection.
///
/// A daemon that exits without unlinking its socket leaves a stale file
/// behind; connecting to it fails (typically `ECONNREFUSED`), and the next
/// candidate is tried, so a stale user socket never shadows a healthy
/// system daemon. The last connection error is returned when every
/// candidate fails.
#[tracing::instrument(skip(candidates), fields(candidate_count = candidates.len()))]
async fn connect_first(candidates: &[PathBuf]) -> Result<UnixStream, ClientError> {
    let mut last_error: Option<std::io::Error> = None;
    for (index, candidate) in candidates.iter().enumerate() {
        let attempt = index + 1;
        debug!(attempt, socket = %candidate.display(), "probing socket candidate");
        match UnixStream::connect(candidate).await {
            Ok(stream) => {
                debug!(attempt, socket = %candidate.display(), "socket candidate selected");
                return Ok(stream);
            }
            Err(error) => {
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) {
                    debug!(attempt, socket = %candidate.display(), error_kind = ?error.kind(), error = %error, "socket candidate unavailable");
                } else {
                    warn!(attempt, socket = %candidate.display(), error_kind = ?error.kind(), error = %error, "socket candidate failed");
                }
                last_error = Some(error);
            }
        }
    }
    Err(ClientError::Connect(last_error.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no socket candidates to try")
    })))
}

/// Send `request` to the first reachable candidate and parse its bounded reply.
async fn transact(candidates: &[PathBuf], request: &Request) -> Result<Response, ClientError> {
    transact_with_timeout(candidates, request, CLIENT_REPLY_TIMEOUT).await
}

/// Send one request and require the daemon to finish its reply within `timeout`.
async fn transact_with_timeout(
    candidates: &[PathBuf],
    request: &Request,
    timeout: Duration,
) -> Result<Response, ClientError> {
    let payload = serde_json::to_vec(request)?;
    let mut stream = connect_first(candidates).await?;
    stream
        .write_all(&payload)
        .await
        .map_err(ClientError::Write)?;
    stream.shutdown().await.map_err(ClientError::Shutdown)?;
    let mut reply = Vec::with_capacity(8 * 1024);
    let mut limited = stream.take((MAX_RESPONSE_BYTES as u64) + 1);
    tokio::time::timeout(timeout, limited.read_to_end(&mut reply))
        .await
        .map_err(|_| ClientError::ReplyTimeout)?
        .map_err(ClientError::Read)?;
    if reply.len() > MAX_RESPONSE_BYTES {
        return Err(ClientError::ReplyTooLarge);
    }
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
    let stdout = std::io::stdout();
    run_with_writer(args, &mut stdout.lock()).await
}

/// Execute the parsed command and render its result through `writer`.
async fn run_with_writer<W: Write>(args: Args, writer: &mut W) -> Result<(), ClientError> {
    let request = args.command.to_request();
    let response = transact(&args.socket_candidates(), &request).await?;
    let (entry, entries) = match response {
        Response::Error { message } => return Err(ClientError::Daemon(message)),
        Response::Ok { entry, entries } => (entry, entries),
    };
    render_response(&args.command, entry, entries, writer)
}

/// Render a response whose shape has already been checked against `command`.
fn render_response<W: Write>(
    command: &Command,
    entry: Option<comenq_lib::protocol::PendingEntry>,
    entries: Option<Vec<comenq_lib::protocol::PendingEntry>>,
    writer: &mut W,
) -> Result<(), ClientError> {
    match (command, entry, entries) {
        (Command::Put { .. }, Some(entry), None) => {
            let _ = write_line(writer, &render_put(&entry))?;
        }
        (Command::List, None, Some(entries)) => {
            if entries.is_empty() {
                let _ = write_line(writer, "No comments queued.")?;
            } else {
                for entry in entries {
                    if !write_line(writer, &render_entry(&entry))? {
                        break;
                    }
                }
            }
        }
        (Command::Bump { id }, None, None) => {
            let _ = write_line(writer, &format!("Moved {id} to the head of the queue."))?;
        }
        (Command::Bust { id }, None, None) => {
            let _ = write_line(writer, &format!("Moved {id} to the tail of the queue."))?;
        }
        (Command::Del { id }, None, None) => {
            let _ = write_line(writer, &format!("Removed {id} from the queue."))?;
        }
        _ => return Err(ClientError::UnexpectedResponse),
    }
    Ok(())
}

/// Write one line, treating a closed output pipe as successful completion.
fn write_line(writer: &mut impl Write, line: &str) -> Result<bool, ClientError> {
    match writeln!(writer, "{line}") {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(ClientError::Output(error)),
    }
}

#[cfg(test)]
mod tests;
