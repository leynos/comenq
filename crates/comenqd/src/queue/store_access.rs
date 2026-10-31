//! Serialized store work, await projection caching, and operation metrics.

use super::SharedQueue;
use super::await_lookup::{AwaitSchedule, is_state_mutation};
use super::{QueueResult, UnixClock, queue_error_category};
use crate::metrics;
use crate::store::{QueueStore, Result as StoreResult, StoreError};
use comenq_domain::{FlutterSampler, QueueError, QueuePolicy};
use std::sync::Arc;
use std::time::Instant;

/// Store and ephemeral await projections share one mutation boundary.
pub(super) struct QueueState {
    pub(super) store: QueueStore,
    pub(super) await_schedule: Option<AwaitSchedule>,
    #[cfg(test)]
    pub(super) projection_builds: u64,
}

impl std::fmt::Debug for QueueState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueueState")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

impl QueueState {
    /// Begin without a cached schedule.
    pub(super) fn new(store: QueueStore) -> Self {
        Self {
            store,
            await_schedule: None,
            #[cfg(test)]
            projection_builds: 0,
        }
    }
}

/// Failure type accepted by the serialized store boundary.
pub(super) trait StoreFailure: std::fmt::Display + Send + 'static {
    /// Return a bounded classification that does not expose stored values.
    fn category(&self) -> &'static str;

    /// Whether this error represents an unexpected persistence or task failure.
    fn is_unexpected(&self) -> bool;

    /// Wrap a blocking-task join failure in this error type.
    fn from_join(error: tokio::task::JoinError) -> Self;
}

impl StoreFailure for StoreError {
    fn category(&self) -> &'static str {
        StoreError::category(self)
    }

    fn is_unexpected(&self) -> bool {
        StoreError::is_unexpected(self)
    }

    fn from_join(error: tokio::task::JoinError) -> Self {
        StoreError::BlockingTask(error)
    }
}

impl StoreFailure for QueueError<StoreError> {
    fn category(&self) -> &'static str {
        queue_error_category(self)
    }

    fn is_unexpected(&self) -> bool {
        self.repository().is_some_and(StoreError::is_unexpected)
    }

    fn from_join(error: tokio::task::JoinError) -> Self {
        QueueError::Repository(StoreError::BlockingTask(error))
    }
}

impl SharedQueue {
    /// Adapt existing store operations to the shared state boundary.
    pub(super) async fn with_store<T, F>(&self, operation: &'static str, work: F) -> StoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&QueueStore) -> StoreResult<T> + Send + 'static,
    {
        self.with_state(operation, move |state| work(&state.store))
            .await
    }

    /// Run synchronous work while holding the queue lock and domain ports.
    pub(super) async fn with_service<T, F>(
        &self,
        operation: &'static str,
        work: F,
    ) -> QueueResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&QueueStore, &dyn UnixClock, &dyn FlutterSampler, QueuePolicy) -> QueueResult<T>
            + Send
            + 'static,
    {
        let clock = Arc::clone(&self.clock);
        let sampler = Arc::clone(&self.flutter_sampler);
        let policy = self.policy;
        self.with_state(operation, move |state| {
            work(&state.store, clock.as_ref(), sampler.as_ref(), policy)
        })
        .await
    }

    /// Run synchronous store work outside Tokio's asynchronous executor.
    pub(super) async fn with_state<T, E, F>(&self, operation: &'static str, work: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: StoreFailure,
        F: FnOnce(&mut QueueState) -> Result<T, E> + Send + 'static,
    {
        let started = Instant::now();
        let store = Arc::clone(&self.store);
        let state_version = Arc::clone(&self.state_version);
        let span = tracing::info_span!(
            "queue_store",
            operation,
            elapsed_ms = tracing::field::Empty,
            outcome = tracing::field::Empty,
            error_kind = tracing::field::Empty,
        );
        let worker_span = span.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _entered = worker_span.enter();
            let mut state = store
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let result = work(&mut state);
            if result.is_ok() && is_state_mutation(operation) {
                if state.await_schedule.take().is_some() {
                    metrics::record_await_schedule_cache("invalidation");
                }
                // Publish under the same lock even if the calling future was dropped.
                state_version.send_modify(|version| *version = version.wrapping_add(1));
            }
            result
        })
        .await
        .unwrap_or_else(|error| Err(E::from_join(error)));
        let duration = started.elapsed();
        let elapsed_ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        span.record("elapsed_ms", elapsed_ms);
        let (outcome, error_kind) = match &result {
            Ok(_) => {
                span.record("outcome", "success");
                span.record("error_kind", "none");
                ("success", None)
            }
            Err(error) => {
                let error_kind = error.category();
                span.record("outcome", "failure");
                span.record("error_kind", error_kind);
                if error.is_unexpected() {
                    tracing::error!(
                        parent: &span,
                        operation,
                        error_kind,
                        "Queue store operation failed",
                    );
                }
                ("failure", Some(error_kind))
            }
        };
        metrics::record_queue_store_duration(operation, outcome, error_kind, duration);
        result
    }

    /// Refresh entry-count and accounted-byte gauges from persisted data.
    pub(super) async fn update_queue_gauges(&self) {
        match self
            .with_store("queue_metrics_snapshot", QueueStore::queue_metrics_snapshot)
            .await
        {
            Ok((count, bytes)) => {
                metrics::record_queue_entries(count);
                metrics::record_queue_bytes(bytes);
            }
            Err(_) => tracing::warn!(
                error_kind = "queue_snapshot_failed",
                "Failed to refresh queue metrics"
            ),
        }
    }
}
