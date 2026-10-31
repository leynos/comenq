//! Client-side communication with the `comenqd` daemon.
//!
//! This module serializes protocol requests, sends them to the daemon over
//! its Unix Domain Socket, and renders the replies. It is separated from
//! `lib.rs` so that argument parsing remains focused and the network logic
//! is easily testable.

use comenq_lib::protocol::{
    HistoryEntry, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, Request, Response,
};
use mockable::{DefaultEnv, Env};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};
use tracing::{debug, warn};

use crate::output::{render_entry, render_history as render_history_line, render_put};
use crate::{Args, Command};

mod await_stream;
mod error;
#[doc(hidden)]
pub use await_stream::await_with_writers;
pub use error::ClientError;

/// Maximum time to wait for a complete daemon reply.
const CLIENT_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

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
    if payload.len() > MAX_REQUEST_BYTES {
        return Err(ClientError::RequestTooLarge {
            size: payload.len(),
            limit: MAX_REQUEST_BYTES,
        });
    }
    tokio::time::timeout(timeout, async {
        let mut stream = connect_first(candidates).await?;
        stream
            .write_all(&payload)
            .await
            .map_err(ClientError::Write)?;
        stream.shutdown().await.map_err(ClientError::Shutdown)?;
        let mut reply = Vec::with_capacity(8 * 1024);
        let mut limited = stream.take((MAX_RESPONSE_BYTES as u64) + 1);
        limited
            .read_to_end(&mut reply)
            .await
            .map_err(ClientError::Read)?;
        if reply.len() > MAX_RESPONSE_BYTES {
            return Err(ClientError::ReplyTooLarge);
        }
        Ok(serde_json::from_slice(&reply)?)
    })
    .await
    .map_err(|_| ClientError::ReplyTimeout)?
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
    run_with_env(args, &DefaultEnv).await
}

/// Execute a parsed command using the supplied environment provider.
///
/// Resolve socket candidates before the first asynchronous operation so the
/// environment provider is only needed during configuration.
///
/// # Examples
///
/// ```no_run
/// # use comenq::{Args, Command, run_with_env};
/// # use mockable::DefaultEnv;
/// # use std::path::PathBuf;
/// # async fn try_run() -> Result<(), comenq::ClientError> {
/// let args = Args {
///     socket: Some(PathBuf::from("/tmp/comenq.sock")),
///     command: Command::List,
/// };
/// run_with_env(args, &DefaultEnv).await?;
/// # Ok(())
/// # }
/// ```
pub async fn run_with_env(args: Args, env: &dyn Env) -> Result<(), ClientError> {
    let candidates = args.socket_candidates_with_env(env);
    if let Command::Await { id } = &args.command {
        return await_with_writers(
            &candidates,
            id,
            &mut std::io::stdout(),
            &mut std::io::stderr(),
        )
        .await;
    }
    if let Command::Hist { limit } = &args.command {
        return run_history_pages(&candidates, *limit, |history, now| {
            let stdout = std::io::stdout();
            render_history_reply(history, now, &mut stdout.lock())
        })
        .await;
    }
    let response = transact(&candidates, &args.command.to_request()).await?;
    let stdout = std::io::stdout();
    render_reply(&args.command, response, unix_now(), &mut stdout.lock())
}

/// Execute the parsed command and render its result through `writer`.
///
/// A closed output pipe stops rendering successfully so commands can be used
/// in pipelines whose downstream consumer exits early.
#[cfg(test)]
async fn run_with_writer<W: Write>(args: Args, writer: &mut W) -> Result<(), ClientError> {
    let candidates = args.socket_candidates();
    if let Command::Await { id } = &args.command {
        return await_with_writers(&candidates, id, writer, &mut std::io::sink()).await;
    }
    if let Command::Hist { limit } = &args.command {
        return run_history_pages(&candidates, *limit, |history, now| {
            render_history_reply(history, now, writer)
        })
        .await;
    }
    let request = args.command.to_request();
    let response = transact(&candidates, &request).await?;
    render_reply(&args.command, response, unix_now(), writer)
}

async fn run_history_pages(
    candidates: &[PathBuf],
    limit: Option<usize>,
    mut render_page: impl FnMut(&[HistoryEntry], u64) -> Result<bool, ClientError>,
) -> Result<(), ClientError> {
    let now = unix_now();
    let mut cursor = None;
    let mut found_records = false;
    loop {
        let request = Request::Hist {
            limit: cursor.is_none().then_some(limit).flatten(),
            cursor,
        };
        let response = transact(candidates, &request).await?;
        let (history, next_cursor) = match response {
            Response::Error { message } => return Err(ClientError::Daemon(message)),
            Response::Ok {
                entry: None,
                entries: None,
                history: Some(history),
                next_cursor,
            } => (history, next_cursor),
            _ => return Err(ClientError::UnexpectedResponse),
        };
        if !history.is_empty() {
            found_records = true;
            if !render_page(&history, now)? {
                return Ok(());
            }
        }
        if cursor.is_some() && next_cursor == cursor {
            return Err(ClientError::UnexpectedResponse);
        }
        cursor = next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    if !found_records {
        let _ = render_page(&[], now)?;
    }
    Ok(())
}

/// Check the daemon result and render it without holding a writer across I/O.
fn render_reply<W: Write>(
    command: &Command,
    response: Response,
    now: u64,
    writer: &mut W,
) -> Result<(), ClientError> {
    let (entry, entries, history) = match response {
        Response::Error { message } => return Err(ClientError::Daemon(message)),
        Response::Ok {
            entry,
            entries,
            history,
            ..
        } => (entry, entries, history),
    };
    render_response(command, entry, entries, history, now, writer)
}

/// Render a response whose shape has already been checked against `command`.
fn render_response<W: Write>(
    command: &Command,
    entry: Option<comenq_lib::protocol::PendingEntry>,
    entries: Option<Vec<comenq_lib::protocol::PendingEntry>>,
    history: Option<Vec<HistoryEntry>>,
    now: u64,
    writer: &mut W,
) -> Result<(), ClientError> {
    match (command, entry, entries, history) {
        (Command::Put { .. }, Some(entry), None, None) => {
            let _ = write_line(writer, &render_put(&entry))?;
        }
        (Command::List, None, Some(entries), None) => {
            render_list_reply(entries, writer)?;
        }
        (Command::Bump { id }, None, None, None) => {
            let _ = write_line(writer, &format!("Moved {id} to the head of the queue."))?;
        }
        (Command::Bust { id }, None, None, None) => {
            let _ = write_line(writer, &format!("Moved {id} to the tail of the queue."))?;
        }
        (Command::Del { id }, None, None, None) => {
            let _ = write_line(writer, &format!("Removed {id} from the queue."))?;
        }
        (Command::Hist { .. }, None, None, Some(history)) => {
            render_history_reply(&history, now, writer)?;
        }
        _ => return Err(ClientError::UnexpectedResponse),
    }
    Ok(())
}

/// Render the pending entries returned by `list`.
fn render_list_reply<W: Write>(
    entries: Vec<comenq_lib::protocol::PendingEntry>,
    writer: &mut W,
) -> Result<(), ClientError> {
    if entries.is_empty() {
        let _ = write_line(writer, "No comments queued.")?;
    } else {
        for entry in entries {
            if !write_line(writer, &render_entry(&entry))? {
                break;
            }
        }
    }
    Ok(())
}

/// Render the posting attempts returned by `hist`.
fn render_history_reply<W: Write>(
    history: &[HistoryEntry],
    now: u64,
    writer: &mut W,
) -> Result<bool, ClientError> {
    if history.is_empty() {
        return write_line(writer, "No posting history recorded.");
    }
    render_history_entries(history, now, writer)
}

fn render_history_entries<W: Write>(
    history: &[HistoryEntry],
    now: u64,
    writer: &mut W,
) -> Result<bool, ClientError> {
    for record in history {
        if !write_line(writer, &render_history_line(record, now))? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
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
