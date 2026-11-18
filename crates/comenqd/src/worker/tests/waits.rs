//! Focused tests for the worker's shutdown-aware wait helpers.
//!
//! Each helper is driven with single manual polls so every branch is observed
//! exactly once. Tokio time is paused, so a deadline only passes when a test
//! advances the clock explicitly; buffered notifications and sender drops then
//! resolve the remaining branches without a real wake-up.

use crate::config::Config;
use crate::queue::SharedQueue;
use crate::worker::{WorkerControl, wait_for_retry_deadline, wait_for_scheduled_entry};
use rstest::rstest;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use tempfile::{TempDir, tempdir};
use test_support::temp_config;
use tokio::sync::{Notify, watch};
use tokio::time::Instant;

/// Poll `future` once with a no-op waker.
///
/// Paused time and already-buffered notifications make each covered branch
/// resolve on a single poll, so no executor or real wake-up is required.
fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

/// How a retry-deadline wait should observe shutdown.
#[derive(Clone, Copy, Debug)]
enum RetryShutdownTrigger {
    /// Signal shutdown after the future first reports `Pending`.
    SignalAfterPending,
    /// Signal shutdown before the first poll, leaving a buffered change.
    BufferedSignalBeforeFirst,
    /// Drop the sender after the future first reports `Pending`.
    DropSenderAfterPending,
}

/// Every shutdown route must end the retry wait with `true`.
///
/// A dropped watch sender is treated as shutdown because
/// [`watch::Receiver::changed`] reports the closed channel immediately; the
/// worker must then stop rather than keep retrying while nobody can signal it.
#[rstest]
#[case::send_after_pending_poll(RetryShutdownTrigger::SignalAfterPending)]
#[case::buffered_send_before_poll(RetryShutdownTrigger::BufferedSignalBeforeFirst)]
#[case::drop_sender_after_pending_poll(RetryShutdownTrigger::DropSenderAfterPending)]
#[tokio::test(start_paused = true)]
async fn retry_wait_returns_true_for_each_shutdown_trigger(#[case] trigger: RetryShutdownTrigger) {
    let changed = Notify::new();
    let (shutdown_tx, mut shutdown_rx) = watch::channel(());
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut wait = std::pin::pin!(wait_for_retry_deadline(
        deadline,
        &changed,
        &mut shutdown_rx
    ));

    match trigger {
        RetryShutdownTrigger::BufferedSignalBeforeFirst => {
            shutdown_tx.send(()).expect("signal shutdown");
            assert!(matches!(poll_once(wait.as_mut()), Poll::Ready(true)));
        }
        RetryShutdownTrigger::SignalAfterPending => {
            assert!(matches!(poll_once(wait.as_mut()), Poll::Pending));
            shutdown_tx.send(()).expect("signal shutdown");
            assert!(matches!(poll_once(wait.as_mut()), Poll::Ready(true)));
        }
        RetryShutdownTrigger::DropSenderAfterPending => {
            assert!(matches!(poll_once(wait.as_mut()), Poll::Pending));
            drop(shutdown_tx);
            assert!(matches!(poll_once(wait.as_mut()), Poll::Ready(true)));
        }
    }
}

/// Where a retry deadline sits relative to paused "now".
#[derive(Clone, Copy, Debug)]
enum DeadlinePosition {
    /// The deadline is exactly `Instant::now()`.
    Now,
    /// The deadline elapsed one second before the wait begins.
    AlreadyPassed,
}

/// A deadline that has already arrived returns `false` on the first poll.
///
/// This replaces the former zero-duration `wait_or_shutdown` contract: no
/// timer is armed and no shutdown state is inspected once the remaining time
/// is zero.
#[rstest]
#[case::deadline_is_now(DeadlinePosition::Now)]
#[case::deadline_already_passed(DeadlinePosition::AlreadyPassed)]
#[tokio::test(start_paused = true)]
async fn retry_wait_returns_false_when_the_deadline_has_passed(#[case] position: DeadlinePosition) {
    let changed = Notify::new();
    let (_shutdown_tx, mut shutdown_rx) = watch::channel(());
    let deadline = match position {
        DeadlinePosition::Now => Instant::now(),
        DeadlinePosition::AlreadyPassed => Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("paused clock is far enough from its epoch"),
    };
    let mut wait = std::pin::pin!(wait_for_retry_deadline(
        deadline,
        &changed,
        &mut shutdown_rx
    ));

    assert!(matches!(poll_once(wait.as_mut()), Poll::Ready(false)));
}

/// A live wait stays pending until its positive timeout elapses.
#[tokio::test(start_paused = true)]
async fn retry_wait_reports_pending_until_the_timeout_elapses() {
    let changed = Notify::new();
    let (_shutdown_tx, mut shutdown_rx) = watch::channel(());
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut wait = std::pin::pin!(wait_for_retry_deadline(
        deadline,
        &changed,
        &mut shutdown_rx
    ));

    assert!(matches!(poll_once(wait.as_mut()), Poll::Pending));
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(poll_once(wait.as_mut()), Poll::Pending));
    tokio::time::advance(Duration::from_secs(1)).await;
    // Advancing the clock does not guarantee the timer driver has run yet, so
    // await the final wait and let the runtime drive it to completion.
    assert!(!wait.await);
}

/// Shutdown wins even when a queue notification is already buffered.
///
/// The biased `select!` in the helper gives shutdown priority so a queue
/// mutation cannot mask a stop request during the retry cooldown.
#[tokio::test(start_paused = true)]
async fn retry_wait_prioritizes_shutdown_over_queue_notification() {
    let changed = Notify::new();
    let (shutdown_tx, mut shutdown_rx) = watch::channel(());
    let deadline = Instant::now() + Duration::from_secs(60);
    changed.notify_one();
    shutdown_tx.send(()).expect("signal shutdown");
    let mut wait = std::pin::pin!(wait_for_retry_deadline(
        deadline,
        &changed,
        &mut shutdown_rx
    ));

    assert!(matches!(poll_once(wait.as_mut()), Poll::Ready(true)));
}

/// How a scheduled-entry wait should observe shutdown.
#[derive(Clone, Copy, Debug)]
enum ScheduledShutdownTrigger {
    /// Signal shutdown after the future first reports `Pending`.
    SignalAfterPending,
    /// Drop the sender after the future first reports `Pending`.
    SenderDropped,
}

/// Open an empty queue backed by a temporary directory.
///
/// The scheduled-entry wait only borrows the change notifier, so no queue
/// entries or fake GitHub client are needed. The returned [`TempDir`] must be
/// kept alive for the queue store to remain valid.
fn open_queue() -> (TempDir, Arc<SharedQueue>) {
    let dir = tempdir().expect("create tempdir");
    let queue = SharedQueue::open(Arc::new(Config::from(temp_config(&dir))))
        .expect("open queue in tempdir");
    (dir, queue)
}

/// Every shutdown route must end the scheduled-entry wait with `true`.
#[rstest]
#[case::send_after_pending_poll(ScheduledShutdownTrigger::SignalAfterPending)]
#[case::drop_sender_after_pending_poll(ScheduledShutdownTrigger::SenderDropped)]
#[tokio::test(start_paused = true)]
async fn scheduled_wait_returns_true_for_each_shutdown_trigger(
    #[case] trigger: ScheduledShutdownTrigger,
) {
    let (_dir, queue) = open_queue();
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let mut control = WorkerControl::without_hooks(shutdown_rx);
    let mut wait = std::pin::pin!(wait_for_scheduled_entry(&queue, 60, &mut control));

    assert!(matches!(poll_once(wait.as_mut()), Poll::Pending));
    match trigger {
        ScheduledShutdownTrigger::SignalAfterPending => {
            shutdown_tx.send(()).expect("signal shutdown");
        }
        ScheduledShutdownTrigger::SenderDropped => drop(shutdown_tx),
    }
    assert!(matches!(poll_once(wait.as_mut()), Poll::Ready(true)));
}

/// A queue mutation wakes the scheduled-entry wait with `false`.
///
/// Returning `false` tells the worker to recompute the head entry rather than
/// stopping, so the queue change is handled on the next loop iteration.
#[tokio::test(start_paused = true)]
async fn scheduled_wait_returns_false_when_the_queue_changes() {
    let (_dir, queue) = open_queue();
    let (_shutdown_tx, shutdown_rx) = watch::channel(());
    let mut control = WorkerControl::without_hooks(shutdown_rx);
    let mut wait = std::pin::pin!(wait_for_scheduled_entry(&queue, 60, &mut control));

    assert!(matches!(poll_once(wait.as_mut()), Poll::Pending));
    queue.change_notifier().notify_one();
    assert!(matches!(poll_once(wait.as_mut()), Poll::Ready(false)));
}

/// An unchanged queue lets the scheduled-entry timer elapse with `false`.
#[tokio::test(start_paused = true)]
async fn scheduled_wait_returns_false_when_the_timer_elapses() {
    let (_dir, queue) = open_queue();
    let (_shutdown_tx, shutdown_rx) = watch::channel(());
    let mut control = WorkerControl::without_hooks(shutdown_rx);
    let mut wait = std::pin::pin!(wait_for_scheduled_entry(&queue, 2, &mut control));

    assert!(matches!(poll_once(wait.as_mut()), Poll::Pending));
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(poll_once(wait.as_mut()), Poll::Pending));
    tokio::time::advance(Duration::from_secs(1)).await;
    // Advancing the clock does not guarantee the timer driver has run yet, so
    // await the final wait and let the runtime drive it to completion.
    assert!(!wait.await);
}
