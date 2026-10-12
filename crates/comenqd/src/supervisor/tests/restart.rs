//! Loop-level restart and shutdown regressions using controlled task failures.

use super::super::{STABLE_TASK_RUN, supervise_task};
use backon::{BackoffBuilder, ExponentialBuilder};
use rstest::rstest;
use serde_json::Value;
use std::io::Write;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tracing_subscriber::prelude::*;

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

/// Exercise retries through the actual supervisor loop, including a stable reset.
#[rstest]
#[case::transient(Duration::from_secs(3), false)]
#[case::stable(STABLE_TASK_RUN, true)]
fn failed_tasks_restart_and_stable_runs_reset_backoff(
    #[case] run_duration: Duration,
    #[case] expected_reset: bool,
) {
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let subscriber =
        tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().json().with_writer(
            move || EventWriter {
                events: events_tx.clone(),
                bytes: Vec::new(),
            },
        ));
    tracing::subscriber::with_default(subscriber, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .expect("build paused runtime")
            .block_on(async {
                let (_shutdown_tx, shutdown) = watch::channel(());
                let (spawn_tx, mut spawns) = mpsc::unbounded_channel();
                let mut spawn_count = 0;
                let minimum = Duration::from_millis(100);
                let restart_backoff = ExponentialBuilder::default()
                    .with_min_delay(minimum)
                    .without_max_times()
                    .build();
                let supervisor = supervise_task(
                    "worker",
                    tokio::spawn(async { Err(anyhow::anyhow!("initial failure")) }),
                    restart_backoff,
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
                    },
                    shutdown,
                    minimum,
                );
                let driver = async {
                    let first = receive(&mut events).await;
                    assert_eq!(first["task"], "worker");
                    assert_eq!(first["restart_attempt"], 1);
                    assert_eq!(first["backoff_reset"], false);
                    assert_eq!(first["selected_delay_ms"], 100);
                    tokio::time::advance(Duration::from_millis(99)).await;
                    assert!(spawns.try_recv().is_err(), "retry must respect its delay");
                    tokio::time::advance(Duration::from_millis(1)).await;
                    let fail = receive(&mut spawns).await;
                    tokio::time::advance(run_duration).await;
                    fail.send(())
                        .expect("fail replacement after controlled run");
                    let second = receive(&mut events).await;
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
                    receive(&mut spawns).await;
                };
                tokio::time::timeout(Duration::from_secs(90), async {
                    tokio::join!(supervisor, driver);
                })
                .await
                .expect("supervisor and controlled driver finish");
                assert!(
                    spawns.try_recv().is_err(),
                    "normal completion must not respawn"
                );
            });
    });
}

/// Shutdown during the restart delay must stop the loop without spawning again.
#[test]
fn shutdown_cancels_pending_restart() {
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let subscriber =
        tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().json().with_writer(
            move || EventWriter {
                events: events_tx.clone(),
                bytes: Vec::new(),
            },
        ));
    tracing::subscriber::with_default(subscriber, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .expect("build paused runtime")
            .block_on(async {
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
            });
    });
}
