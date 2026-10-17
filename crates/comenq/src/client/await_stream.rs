//! Capped NDJSON transport and foreground countdown reporting.

use super::{CLIENT_REPLY_TIMEOUT, ClientError, connect_first, write_line};
use crate::output::{render_remaining, report_interval, terminal_safe};
use comenq_lib::protocol::{
    AWAIT_IDLE_TIMEOUT_SECS, AwaitEvent, MAX_AWAIT_FRAME_BYTES, MAX_REQUEST_BYTES, Request,
};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::time::Instant;

/// Wait for a queued comment, writing countdowns to `stderr` and its URL to `stdout`.
///
/// Cancelling this future only disconnects the client; it never changes the queue.
///
/// # Examples
///
/// ```rust,no_run
/// # async fn example() -> Result<(), comenq::ClientError> {
/// let mut stdout = std::io::stdout();
/// let mut stderr = std::io::stderr();
/// comenq::client::await_with_writers(
///     &["/tmp/comenq.sock".into()], "1a2b3c4d", &mut stdout, &mut stderr,
/// ).await?;
/// # Ok(())
/// # }
/// ```
#[doc(hidden)]
pub async fn await_with_writers(
    candidates: &[PathBuf],
    id: &str,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<(), ClientError> {
    let bytes = serde_json::to_vec(&Request::Await { id: id.to_owned() })?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(ClientError::RequestTooLarge {
            size: bytes.len(),
            limit: MAX_REQUEST_BYTES,
        });
    }
    let mut stream = tokio::time::timeout(CLIENT_REPLY_TIMEOUT, connect_first(candidates))
        .await
        .map_err(|_| ClientError::ReplyTimeout)??;
    tokio::time::timeout(CLIENT_REPLY_TIMEOUT, async {
        stream.write_all(&bytes).await.map_err(ClientError::Write)?;
        stream.shutdown().await.map_err(ClientError::Shutdown)
    })
    .await
    .map_err(|_| ClientError::ReplyTimeout)??;
    receive(&mut BufReader::new(stream), stdout, stderr).await
}

struct Countdown {
    eta: u64,
    arrived: Instant,
    next_report: Instant,
}

impl Countdown {
    /// Subtract time elapsed since the latest ETA frame, saturating at zero.
    fn remaining(&self) -> u64 {
        self.eta.saturating_sub(self.arrived.elapsed().as_secs())
    }

    /// Write one countdown line and schedule the next cadence tick.
    fn report(&mut self, stderr: &mut impl Write) -> Result<(), ClientError> {
        let remaining = self.remaining();
        let _ = write_line(stderr, &render_remaining(remaining))?;
        self.next_report = Instant::now() + Duration::from_secs(report_interval(remaining));
        Ok(())
    }

    /// Start a countdown on its first frame and refresh its ETA on later frames.
    fn observe(
        countdown: &mut Option<Self>,
        eta_seconds: u64,
        stderr: &mut impl Write,
    ) -> Result<(), ClientError> {
        if let Some(current) = countdown {
            current.eta = eta_seconds;
            current.arrived = Instant::now();
            current.next_report = current
                .next_report
                .min(Instant::now() + Duration::from_secs(report_interval(eta_seconds)));
        } else {
            let mut first = Self {
                eta: eta_seconds,
                arrived: Instant::now(),
                next_report: Instant::now(),
            };
            first.report(stderr)?;
            *countdown = Some(first);
        }
        Ok(())
    }
}

/// Consume stream frames until a comment URL arrives or the daemon reports an error.
pub(super) async fn receive(
    reader: &mut (impl AsyncBufRead + Unpin),
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<(), ClientError> {
    let mut countdown: Option<Countdown> = None;
    let mut deadline = CLIENT_REPLY_TIMEOUT;
    loop {
        let event = next_event(reader, &mut countdown, stderr, deadline).await?;
        deadline = Duration::from_secs(AWAIT_IDLE_TIMEOUT_SECS);
        match event {
            AwaitEvent::Posted { comment_url } => {
                let _ = write_line(stdout, &terminal_safe(&comment_url))?;
                return Ok(());
            }
            AwaitEvent::Error { message } => return Err(ClientError::Daemon(message)),
            AwaitEvent::Waiting { eta_seconds } => {
                Countdown::observe(&mut countdown, eta_seconds, stderr)?;
            }
        }
    }
}

/// Read the next event while preserving partial frames across countdown ticks.
async fn next_event(
    reader: &mut (impl AsyncBufRead + Unpin),
    countdown: &mut Option<Countdown>,
    stderr: &mut impl Write,
    deadline: Duration,
) -> Result<AwaitEvent, ClientError> {
    // Keep the same read future alive across countdown ticks: cancelling a
    // partially consumed frame would lose its prefix.
    let frame = tokio::time::timeout(deadline, read_frame(reader));
    tokio::pin!(frame);
    loop {
        let report_at = countdown.as_ref().map(|countdown| countdown.next_report);
        tokio::select! {
            biased;
            result = &mut frame => return result.map_err(|_| ClientError::ReplyTimeout)?,
            () = report_tick(report_at) => {
                if let Some(countdown) = countdown { countdown.report(stderr)?; }
            }
        }
    }
}

/// Wait until the countdown's next report, or forever before the first ETA.
async fn report_tick(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending::<()>().await,
    }
}

/// Read and validate one capped newline-delimited await event.
async fn read_frame(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<AwaitEvent, ClientError> {
    let mut bytes = Vec::new();
    let mut limited = (&mut *reader).take((MAX_AWAIT_FRAME_BYTES + 1) as u64);
    limited
        .read_until(b'\n', &mut bytes)
        .await
        .map_err(ClientError::Read)?;
    if bytes.len() > MAX_AWAIT_FRAME_BYTES {
        return Err(ClientError::ReplyTooLarge);
    }
    if bytes.last() != Some(&b'\n') {
        return Err(ClientError::ConnectionLost);
    }
    serde_json::from_slice(&bytes).map_err(|_| ClientError::UnexpectedResponse)
}
