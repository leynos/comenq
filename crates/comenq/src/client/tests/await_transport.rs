//! Streaming await transport, output separation, and deadline regressions.

use super::super::{ClientError, await_stream::receive, await_with_writers, run_with_writer};
use crate::{Args, Command};
use comenq_lib::protocol::{AwaitEvent, MAX_AWAIT_FRAME_BYTES, Request};
use rstest::rstest;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

const URL: &str = "https://github.com/o/r/issues/1#issuecomment-1";

fn frames(events: &[AwaitEvent]) -> Result<Vec<u8>, ClientError> {
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend(serde_json::to_vec(event)?);
        bytes.push(b'\n');
    }
    Ok(bytes)
}

#[tokio::test]
async fn sends_request_and_separates_countdown_from_url() -> Result<(), ClientError> {
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("socket");
    let listener = UnixListener::bind(&socket)?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut request = Vec::new();
        stream.read_to_end(&mut request).await?;
        let request = serde_json::from_slice::<Request>(&request)?;
        stream
            .write_all(&frames(&[
                AwaitEvent::Waiting { eta_seconds: 150 },
                AwaitEvent::Waiting { eta_seconds: 60 },
                AwaitEvent::Posted {
                    comment_url: URL.into(),
                },
            ])?)
            .await?;
        Ok::<_, ClientError>(request)
    });
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    await_with_writers(&[socket], "1a2b3c4d", &mut stdout, &mut stderr).await?;
    assert_eq!(
        server.await.expect("server completes")?,
        Request::Await {
            id: "1a2b3c4d".into()
        }
    );
    assert_eq!(stdout, format!("{URL}\n").as_bytes());
    assert_eq!(stderr, b"2m 30s remaining\n");
    Ok(())
}

#[tokio::test]
async fn run_dispatches_await_to_streaming_transport() -> Result<(), ClientError> {
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("socket");
    let listener = UnixListener::bind(&socket)?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut request = Vec::new();
        stream.read_to_end(&mut request).await?;
        let request = serde_json::from_slice::<Request>(&request)?;
        stream
            .write_all(&frames(&[AwaitEvent::Posted {
                comment_url: URL.into(),
            }])?)
            .await?;
        Ok::<_, ClientError>(request)
    });
    let args = Args {
        socket: Some(socket),
        command: Command::Await {
            id: "1a2b3c4d".into(),
        },
    };
    let mut stdout = Vec::new();

    run_with_writer(args, &mut stdout).await?;

    assert_eq!(
        server.await.expect("server completes")?,
        Request::Await {
            id: "1a2b3c4d".into()
        }
    );
    assert_eq!(stdout, format!("{URL}\n").as_bytes());
    Ok(())
}

#[rstest]
#[case::unknown(b"{\"status\":\"error\",\"message\":\"unknown identifier\"}\n".to_vec(), "daemon")]
#[case::eof(Vec::new(), "lost")]
#[case::partial(b"{\"status\":\"waiting\"}".to_vec(), "lost")]
#[case::malformed(b"not json\n".to_vec(), "unexpected")]
#[case::oversized(vec![b'x'; MAX_AWAIT_FRAME_BYTES + 1], "large")]
#[tokio::test]
async fn rejects_incomplete_or_invalid_frames(#[case] bytes: Vec<u8>, #[case] expected: &str) {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let error = receive(&mut bytes.as_slice(), &mut stdout, &mut stderr)
        .await
        .expect_err("invalid stream");
    assert!(match expected {
        "daemon" => matches!(error, ClientError::Daemon(_)),
        "lost" => matches!(error, ClientError::ConnectionLost),
        "unexpected" => matches!(error, ClientError::UnexpectedResponse),
        "large" => matches!(error, ClientError::ReplyTooLarge),
        _ => false,
    });
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}

struct BrokenPipe;
impl Write for BrokenPipe {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Default)]
struct InspectableWriter(Arc<Mutex<Vec<u8>>>);

impl InspectableWriter {
    fn contents(&self) -> Vec<u8> {
        self.0.lock().expect("writer lock").clone()
    }
}

impl Write for InspectableWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("writer lock poisoned"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn broken_output_pipes_do_not_fail_a_successful_post() -> Result<(), ClientError> {
    let bytes = frames(&[
        AwaitEvent::Waiting { eta_seconds: 10 },
        AwaitEvent::Posted {
            comment_url: URL.into(),
        },
    ])?;
    receive(&mut bytes.as_slice(), &mut BrokenPipe, &mut BrokenPipe).await
}

#[rstest]
#[case::first(false, 5)]
#[case::idle(true, 45)]
#[tokio::test(start_paused = true)]
async fn enforces_first_and_idle_deadlines(
    #[case] waiting: bool,
    #[case] seconds: u64,
) -> Result<(), ClientError> {
    let (reader, mut writer) = tokio::io::duplex(1024);
    if waiting {
        writer
            .write_all(&frames(&[AwaitEvent::Waiting { eta_seconds: 60 }])?)
            .await?;
    }
    let mut reader = BufReader::new(reader);
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let future = receive(&mut reader, &mut stdout, &mut stderr);
    tokio::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(std::future::Future::poll(future.as_mut(), &mut context).is_pending());
    tokio::time::advance(Duration::from_secs(seconds)).await;
    assert!(matches!(future.await, Err(ClientError::ReplyTimeout)));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn countdown_ticks_preserve_a_partial_frame() -> Result<(), ClientError> {
    let (reader, mut writer) = tokio::io::duplex(1024);
    writer
        .write_all(&frames(&[AwaitEvent::Waiting { eta_seconds: 30 }])?)
        .await?;
    writer.write_all(b"{\"status\":\"posted\",").await?;
    let mut reader = BufReader::new(reader);
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    {
        let future = receive(&mut reader, &mut stdout, &mut stderr);
        tokio::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(std::future::Future::poll(future.as_mut(), &mut context).is_pending());
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(std::future::Future::poll(future.as_mut(), &mut context).is_pending());
        writer
            .write_all(format!("\"comment_url\":\"{URL}\"}}\n").as_bytes())
            .await?;
        future.await?;
    }
    assert_eq!(stderr, b"30s remaining\n20s remaining\n");
    assert_eq!(stdout, format!("{URL}\n").as_bytes());
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn due_now_countdown_repeats_at_most_once_per_minute() -> Result<(), ClientError> {
    let (reader, mut writer) = tokio::io::duplex(1024);
    writer
        .write_all(&frames(&[AwaitEvent::Waiting { eta_seconds: 0 }])?)
        .await?;
    let mut reader = BufReader::new(reader);
    let (mut stdout, mut stderr) = (InspectableWriter::default(), InspectableWriter::default());
    let observed_stdout = stdout.clone();
    let observed_stderr = stderr.clone();
    let future = receive(&mut reader, &mut stdout, &mut stderr);
    tokio::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(std::future::Future::poll(future.as_mut(), &mut context).is_pending());
    assert_eq!(observed_stderr.contents(), b"due now\n");

    for _ in 0..3 {
        tokio::time::advance(Duration::from_secs(15)).await;
        writer
            .write_all(&frames(&[AwaitEvent::Waiting { eta_seconds: 0 }])?)
            .await?;
        assert!(std::future::Future::poll(future.as_mut(), &mut context).is_pending());
    }

    tokio::time::advance(Duration::from_secs(14)).await;
    assert!(std::future::Future::poll(future.as_mut(), &mut context).is_pending());
    assert_eq!(observed_stderr.contents(), b"due now\n");

    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(std::future::Future::poll(future.as_mut(), &mut context).is_pending());
    assert_eq!(observed_stderr.contents(), b"due now\ndue now\n");

    writer
        .write_all(&frames(&[AwaitEvent::Posted {
            comment_url: URL.into(),
        }])?)
        .await?;
    future.await?;
    assert_eq!(observed_stdout.contents(), format!("{URL}\n").as_bytes());
    Ok(())
}
