//! Streaming daemon waits, queue-state notifications, and waiter admission.

use super::*;
use crate::config::Config;
use crate::listener::{connection::handle_client_with_control, protocol::dispatch_request};
use comenq_lib::{
    CommentRequest,
    protocol::{Request, Response},
};
use tempfile::TempDir;
use test_support::temp_config;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::task::JoinHandle;

const URL: &str = "https://github.com/o/r/issues/1#issuecomment-1";

fn queue(dir: &TempDir) -> crate::store::Result<Arc<SharedQueue>> {
    SharedQueue::open(Arc::new(Config::from(temp_config(dir).with_cooldown(0))))
}

async fn put(queue: &SharedQueue) -> anyhow::Result<String> {
    let response = dispatch_request(
        queue,
        Request::Put {
            request: CommentRequest {
                owner: "o".into(),
                repo: "r".into(),
                pr_number: 1,
                body: "await".into(),
            },
            immediate: true,
        },
    )
    .await;
    match response {
        Response::Ok {
            entry: Some(entry), ..
        } => Ok(entry.id),
        other => anyhow::bail!("put failed: {other:?}"),
    }
}

async fn connect(
    queue: Arc<SharedQueue>,
    id: &str,
    shutdown: watch::Receiver<()>,
    permits: Arc<Semaphore>,
) -> anyhow::Result<(BufReader<UnixStream>, JoinHandle<anyhow::Result<()>>)> {
    let (mut client, server) = UnixStream::pair()?;
    let task = tokio::spawn(handle_client_with_control(server, queue, shutdown, permits));
    client
        .write_all(&serde_json::to_vec(&Request::Await { id: id.into() })?)
        .await?;
    client.shutdown().await?;
    Ok((BufReader::new(client), task))
}

async fn frame(client: &mut BufReader<UnixStream>) -> anyhow::Result<AwaitEvent> {
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(2), client.read_line(&mut line)).await??;
    Ok(serde_json::from_str(&line)?)
}

#[rstest::rstest]
#[case::unknown("deadbeef", "unknown comment identifier")]
#[case::invalid("../badid", "eight hexadecimal characters")]
#[tokio::test]
async fn first_frame_rejects_unknown_or_invalid_id(#[case] id: &str, #[case] message: &str) {
    let dir = TempDir::new().expect("tempdir");
    let queue = queue(&dir).expect("queue");
    let (_shutdown, rx) = watch::channel(());
    let permits = Arc::new(Semaphore::new(1));
    let (mut client, task) = connect(queue, id, rx, permits.clone())
        .await
        .expect("connect");
    let event = frame(&mut client).await.expect("frame");
    assert!(matches!(event, AwaitEvent::Error { message: ref text } if text.contains(message)));
    task.await.expect("join").expect("handler");
    assert_eq!(permits.available_permits(), 1);
}

#[tokio::test]
async fn completion_notifies_all_waiters_and_reports_full_url() {
    let dir = TempDir::new().expect("tempdir");
    let queue = queue(&dir).expect("queue");
    let id = put(&queue).await.expect("put");
    let (_shutdown, rx) = watch::channel(());
    let permits = Arc::new(Semaphore::new(2));
    let (mut first, first_task) = connect(queue.clone(), &id, rx.clone(), permits.clone())
        .await
        .expect("first");
    let (mut second, second_task) = connect(queue.clone(), &id, rx, permits.clone())
        .await
        .expect("second");
    assert!(matches!(
        frame(&mut first).await.expect("waiting"),
        AwaitEvent::Waiting { .. }
    ));
    assert!(matches!(
        frame(&mut second).await.expect("waiting"),
        AwaitEvent::Waiting { .. }
    ));
    let entry = queue.next_due().await.expect("next").expect("entry").0;
    queue.complete(&entry, "hash", URL).await.expect("complete");
    assert_eq!(
        frame(&mut first).await.expect("posted"),
        AwaitEvent::Posted {
            comment_url: URL.into()
        }
    );
    assert_eq!(
        frame(&mut second).await.expect("posted"),
        AwaitEvent::Posted {
            comment_url: URL.into()
        }
    );
    first_task.await.expect("join").expect("handler");
    second_task.await.expect("join").expect("handler");
    assert_eq!(permits.available_permits(), 2);
}

#[rstest::rstest]
#[case::removed(false, "was removed")]
#[case::shutdown(true, "shutting down")]
#[tokio::test]
async fn unrecoverable_waits_send_terminal_error(#[case] stop: bool, #[case] message: &str) {
    let dir = TempDir::new().expect("tempdir");
    let queue = queue(&dir).expect("queue");
    let id = put(&queue).await.expect("put");
    let (shutdown, rx) = watch::channel(());
    let (mut client, task) = connect(queue.clone(), &id, rx, Arc::new(Semaphore::new(1)))
        .await
        .expect("connect");
    assert!(matches!(
        frame(&mut client).await.expect("waiting"),
        AwaitEvent::Waiting { .. }
    ));
    if stop {
        shutdown.send(()).expect("shutdown");
    } else {
        queue.del(&id).await.expect("delete");
    }
    let event = frame(&mut client).await.expect("error");
    assert!(matches!(event, AwaitEvent::Error { message: ref text } if text.contains(message)));
    task.await.expect("join").expect("handler");
}

#[tokio::test]
async fn release_claim_updates_waiters_without_consuming_worker_wakeup() {
    let dir = TempDir::new().expect("tempdir");
    let queue = queue(&dir).expect("queue");
    let mut state = queue.subscribe_state();
    let id = put(&queue).await.expect("put");
    state.changed().await.expect("put state");
    tokio::time::timeout(Duration::from_secs(1), queue.changed())
        .await
        .expect("worker permit remains");
    let (claimed, _) = queue.claim_next_due().await.expect("claim").expect("entry");
    state.changed().await.expect("claim state");
    queue
        .release_claim(&id, claimed.claim_token.as_deref().expect("token"))
        .await
        .expect("release");
    state.changed().await.expect("release state");
    let outcome = queue.resolve_await(&id, None).await.expect("lookup");
    assert!(matches!(outcome, AwaitState::Pending { .. }));
    queue
        .complete(&claimed, "hash", URL)
        .await
        .expect("complete");
    state.changed().await.expect("completion state");
}

#[tokio::test]
async fn disconnected_waiter_releases_capacity_after_heartbeat() {
    let dir = TempDir::new().expect("tempdir");
    let queue = queue(&dir).expect("queue");
    let id = put(&queue).await.expect("put");
    let (_shutdown, rx) = watch::channel(());
    let permits = Arc::new(Semaphore::new(1));
    let (mut first_stream, mut server) = UnixStream::pair().expect("pair");
    first_stream.shutdown().await.expect("half-close");
    let first_queue = queue.clone();
    let first_id = id.clone();
    let first_permits = permits.clone();
    let first_shutdown = rx.clone();
    let task = tokio::spawn(async move {
        let _permit = first_permits.try_acquire_owned().expect("test permit");
        await_entry(
            &mut server,
            &first_queue,
            &first_id,
            first_shutdown,
            Duration::from_millis(50),
        )
        .await
    });
    let mut first = BufReader::new(first_stream);
    assert!(matches!(
        frame(&mut first).await.expect("waiting"),
        AwaitEvent::Waiting { .. }
    ));
    let (mut excess, rejected) = connect(queue.clone(), &id, rx.clone(), permits.clone())
        .await
        .expect("excess");
    assert_eq!(
        frame(&mut excess).await.expect("rejected"),
        AwaitEvent::Error {
            message: "too many concurrent await requests".into()
        }
    );
    rejected.await.expect("join").expect("handler");
    drop(first);
    assert_eq!(task.await.expect("join"), "client_gone");
    assert_eq!(permits.available_permits(), 1);
    assert!(matches!(
        queue
            .resolve_await(&id, None)
            .await
            .expect("queue preserved"),
        AwaitState::Pending { .. }
    ));
}

#[tokio::test]
async fn already_posted_id_returns_url_immediately() {
    let dir = TempDir::new().expect("tempdir");
    let queue = queue(&dir).expect("queue");
    let id = put(&queue).await.expect("put");
    let entry = queue.next_due().await.expect("next").expect("entry").0;
    queue.complete(&entry, "hash", URL).await.expect("complete");
    let (_shutdown, rx) = watch::channel(());
    let (mut client, task) = connect(queue, &id, rx, Arc::new(Semaphore::new(1)))
        .await
        .expect("connect");
    assert_eq!(
        frame(&mut client).await.expect("posted"),
        AwaitEvent::Posted {
            comment_url: URL.into()
        }
    );
    let mut line = String::new();
    let read = client.read_line(&mut line).await.expect("EOF");
    assert_eq!(read, 0);
    task.await.expect("join").expect("handler");
}

#[rstest::rstest]
#[case::storage_fault(false, "invalid_json")]
#[case::oversized_url(true, "posted_url_too_large")]
#[tokio::test]
async fn failure_outcomes_preserve_the_bounded_error_category(
    #[case] oversized: bool,
    #[case] expected_outcome: &str,
) {
    let dir = TempDir::new().expect("tempdir");
    let queue = queue(&dir).expect("queue");
    let id = put(&queue).await.expect("put");
    if oversized {
        let entry = queue.next_due().await.expect("next").expect("entry").0;
        queue
            .complete(&entry, "hash", &"x".repeat(MAX_AWAIT_FRAME_BYTES))
            .await
            .expect("complete");
    } else {
        let path = queue
            .config()
            .queue_path
            .join("entries")
            .join(format!("{id}.json"));
        std::fs::write(path, "bad json").expect("corrupt entry");
    }
    let (_shutdown, rx) = watch::channel(());
    let (client, server) = UnixStream::pair().expect("pair");
    let task = tokio::spawn(handle_await(
        server,
        queue,
        id,
        rx,
        Arc::new(Semaphore::new(1)),
    ));
    let mut reader = BufReader::new(client);
    let event = frame(&mut reader).await.expect("error frame");
    assert!(matches!(event, AwaitEvent::Error { .. }));
    assert_eq!(task.await.expect("join"), expected_outcome);
}
