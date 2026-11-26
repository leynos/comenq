//! Loop-level restart and shutdown regressions using controlled task failures.

use super::super::{STABLE_TASK_RUN, supervise_task};
use backon::{BackoffBuilder, ExponentialBuilder};
use rstest::rstest;
use serde_json::Value;
use std::future::Future;
use std::io::Write;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tracing_subscriber::prelude::*;

type EventReceiver = mpsc::UnboundedReceiver<Value>;
type FailureTrigger = oneshot::Sender<()>;
type TaskHandle = tokio::task::JoinHandle<anyhow::Result<()>>;

/// Collect a complete tracing event before forwarding restart decisions.
struct EventWriter {
    events: mpsc::UnboundedSender<Value>,
    bytes: Vec<u8>,
}

impl Write for EventWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for EventWriter {
    fn drop(&mut self) {
        if let Ok(event) = serde_json::from_slice::<Value>(&self.bytes)
            && event["fields"]["message"] == "Restarting task after failure"
        {
            let _ = self.events.send(event["fields"].clone());
        }
    }
}

/// Require an explicit event rather than inferring readiness from scheduler turns.
async fn receive<T>(receiver: &mut mpsc::UnboundedReceiver<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("event arrives within deadline")
        .expect("event channel remains open")
}

/// Run `scenario` on a paused-clock runtime with restart events captured as JSON.
fn run_paused<Fut>(scenario: impl FnOnce(EventReceiver) -> Fut)
where
    Fut: Future<Output = ()>,
{
    let (events_tx, events) = mpsc::unbounded_channel();
    let subscriber =
        tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().json().with_writer(
            move || EventWriter {
                events: events_tx.clone(),
                bytes: Vec::new(),
            },
        ));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("build paused runtime");
    tracing::subscriber::with_default(subscriber, || runtime.block_on(scenario(events)));
}

/// A respawn function whose first replacement fails when told to and whose
/// later replacements complete normally; each spawn announces its trigger.
fn controlled_spawner(
    spawn_tx: mpsc::UnboundedSender<FailureTrigger>,
) -> impl FnMut() -> TaskHandle {
    let mut spawn_count = 0;
    move || {
        spawn_count += 1;
        let (fail_tx, fail_rx) = oneshot::channel();
        spawn_tx.send(fail_tx).expect("announce replacement");
        if spawn_count == 1 {
            tokio::spawn(async move {
                fail_rx.await.expect("receive controlled failure");
                Err(anyhow::anyhow!("replacement failure"))
            })
        } else {
            tokio::spawn(async { Ok(()) })
        }
    }
}

/// Assert the second restart event reports the expected backoff reset decision.
fn assert_second_restart(second: &Value, run_duration: Duration, expected_reset: bool) {
    assert_eq!(second["restart_attempt"], 2);
    assert_eq!(second["backoff_reset"], expected_reset);
    assert_eq!(
        second["stable_run_duration_ms"],
        run_duration.as_millis() as u64
    );
    let delay_ms = second["selected_delay_ms"]
        .as_u64()
        .expect("selected delay");
    if expected_reset {
        assert!(
            (100..200).contains(&delay_ms),
            "reset selects initial jitter range"
        );
    } else {
        assert_eq!(delay_ms, 200, "transient failure advances backoff");
    }
}

/// Steer the supervisor through two restarts, checking each restart event.
async fn drive_restarts(
    events: &mut EventReceiver,
    spawns: &mut mpsc::UnboundedReceiver<FailureTrigger>,
    run_duration: Duration,
    expected_reset: bool,
) {
    let first = receive(events).await;
    assert_eq!(first["task"], "worker");
    assert_eq!(first["restart_attempt"], 1);
    assert_eq!(first["backoff_reset"], false);
    assert_eq!(first["selected_delay_ms"], 100);
    tokio::time::advance(Duration::from_millis(99)).await;
    assert!(spawns.try_recv().is_err(), "retry must respect its delay");
    tokio::time::advance(Duration::from_millis(1)).await;
    let fail = receive(spawns).await;
    tokio::time::advance(run_duration).await;
    fail.send(())
        .expect("fail replacement after controlled run");
    let second = receive(events).await;
    assert_second_restart(&second, run_duration, expected_reset);
    receive(spawns).await;
}

/// Supervise a failing worker while a driver checks its restart decisions.
async fn restart_scenario(mut events: EventReceiver, run_duration: Duration, expected_reset: bool) {
    let (_shutdown_tx, shutdown) = watch::channel(());
    let (spawn_tx, mut spawns) = mpsc::unbounded_channel();
    let minimum = Duration::from_millis(100);
    let restart_backoff = ExponentialBuilder::default()
        .with_min_delay(minimum)
        .without_max_times()
        .build();
    let supervisor = supervise_task(
        "worker",
        tokio::spawn(async { Err(anyhow::anyhow!("initial failure")) }),
        restart_backoff,
        controlled_spawner(spawn_tx),
        shutdown,
        minimum,
    );
    let driver = drive_restarts(&mut events, &mut spawns, run_duration, expected_reset);
    tokio::time::timeout(Duration::from_secs(90), async {
        tokio::join!(supervisor, driver);
    })
    .await
    .expect("supervisor and controlled driver finish");
    assert!(
        spawns.try_recv().is_err(),
        "normal completion must not respawn"
    );
}

/// Exercise retries through the actual supervisor loop, including a stable reset.
#[rstest]
#[case::transient(Duration::from_secs(3), false)]
#[case::stable(STABLE_TASK_RUN, true)]
fn failed_tasks_restart_and_stable_runs_reset_backoff(
    #[case] run_duration: Duration,
    #[case] expected_reset: bool,
) {
    run_paused(|events| restart_scenario(events, run_duration, expected_reset));
}

/// Cancel a pending restart delay and check the supervisor never respawns.
async fn shutdown_scenario(mut events: EventReceiver) {
    let (shutdown_tx, shutdown) = watch::channel(());
    let minimum = Duration::from_secs(10);
    let mut spawns = 0;
    let supervisor = supervise_task(
        "listener",
        tokio::spawn(async { Err(anyhow::anyhow!("failure")) }),
        ExponentialBuilder::default()
            .with_min_delay(minimum)
            .build(),
        || {
            spawns += 1;
            tokio::spawn(async { Ok(()) })
        },
        shutdown,
        minimum,
    );
    let driver = async {
        let event = receive(&mut events).await;
        assert_eq!(event["restart_attempt"], 1);
        shutdown_tx.send(()).expect("cancel restart delay");
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(supervisor, driver);
    })
    .await
    .expect("shutdown interrupts long backoff promptly");
    assert_eq!(spawns, 0);
}

/// Shutdown during the restart delay must stop the loop without spawning again.
#[test]
fn shutdown_cancels_pending_restart() {
    run_paused(shutdown_scenario);
}
