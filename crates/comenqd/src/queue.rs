//! Shared queue state and queue use-case operations.
//!
//! [`SharedQueue`] bundles the persistent [`QueueStore`] with the daemon
//! configuration and a change signal. The listener maps protocol operations
//! to these typed operations, and the worker waits on the change signal so
//! queue mutations are observed promptly.

use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use comenq_lib::CommentRequest;
use rand::Rng;
use tokio::sync::Notify;

use crate::config::Config;
use crate::metrics;
use crate::store::{PutOptions, QueueStore, Result as StoreResult, StoreError, StoredEntry};

/// Current Unix time in whole seconds.
///
/// Clamps to zero should the system clock report a time before the epoch.
#[must_use]
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Wall clock used for timestamps persisted in the queue store.
pub trait UnixClock: Debug + Send + Sync {
    /// Return whole Unix seconds for queue scheduling.
    fn unix_now(&self) -> u64;
}

/// Provide the bounded random delay added to a newly enqueued comment.
pub(crate) trait FlutterSampler: Debug + Send + Sync {
    /// Sample flutter no greater than the caller's configured maximum.
    fn sample(&self, maximum: u64) -> u64;
}

/// Production flutter source backed by the operating system random generator.
#[derive(Debug)]
struct RandomFlutterSampler;

impl FlutterSampler for RandomFlutterSampler {
    fn sample(&self, maximum: u64) -> u64 {
        if maximum == 0 {
            0
        } else {
            rand::rng().random_range(0..=maximum)
        }
    }
}

/// Queue clock that reads the current system Unix time.
#[derive(Debug)]
struct SystemClock;

impl UnixClock for SystemClock {
    fn unix_now(&self) -> u64 {
        unix_now()
    }
}

/// Queue state shared between the listener and the worker.
#[derive(Debug)]
pub struct SharedQueue {
    cfg: Arc<Config>,
    store: Arc<Mutex<QueueStore>>,
    clock: Arc<dyn UnixClock>,
    flutter_sampler: Arc<dyn FlutterSampler>,
    changed: Notify,
}

impl SharedQueue {
    /// Open the queue store described by `cfg`.
    pub fn open(cfg: Arc<Config>) -> StoreResult<Arc<Self>> {
        Self::open_with_clock(cfg, Arc::new(SystemClock))
    }

    /// Open the queue store using `clock` for persisted scheduling timestamps.
    pub fn open_with_clock(cfg: Arc<Config>, clock: Arc<dyn UnixClock>) -> StoreResult<Arc<Self>> {
        Self::open_with_sources(cfg, clock, Arc::new(RandomFlutterSampler))
    }

    fn open_with_sources(
        cfg: Arc<Config>,
        clock: Arc<dyn UnixClock>,
        flutter_sampler: Arc<dyn FlutterSampler>,
    ) -> StoreResult<Arc<Self>> {
        let store = QueueStore::open(&cfg.queue_path)?;
        let (entry_count, accounted_bytes) = store.queue_metrics_snapshot()?;
        metrics::record_queue_entries(entry_count);
        metrics::record_queue_bytes(accounted_bytes);
        Ok(Arc::new(Self {
            cfg,
            store: Arc::new(Mutex::new(store)),
            clock,
            flutter_sampler,
            changed: Notify::new(),
        }))
    }

    /// Open a queue with controlled clock and flutter sources in tests.
    ///
    /// For example, a sampler fixed at seven seconds makes a 60-second
    /// cooldown project to a 67-second deferred ETA.
    #[cfg(test)]
    pub(crate) fn open_with_clock_and_flutter(
        cfg: Arc<Config>,
        clock: Arc<dyn UnixClock>,
        flutter_sampler: Arc<dyn FlutterSampler>,
    ) -> StoreResult<Arc<Self>> {
        Self::open_with_sources(cfg, clock, flutter_sampler)
    }

    /// The daemon configuration this queue was opened with.
    #[must_use]
    pub fn config(&self) -> &Arc<Config> {
        &self.cfg
    }

    /// Wait until the queue contents change.
    pub async fn changed(&self) {
        self.changed.notified().await;
    }

    /// Expose the change notifier to the worker's retry deadline wait.
    pub(crate) fn change_notifier(&self) -> &Notify {
        &self.changed
    }

    /// The head entry and its estimated seconds-until-post, when any.
    pub async fn next_due(&self) -> StoreResult<Option<(StoredEntry, u64)>> {
        let cooldown = self.cfg.cooldown_period_seconds;
        let now = self.clock.unix_now();
        self.with_store("next_due", move |store| store.next_due(cooldown, now))
            .await
    }

    /// Claim the due entry while holding the store lock.
    pub async fn claim_next_due(&self) -> StoreResult<Option<(StoredEntry, u64)>> {
        let cooldown = self.cfg.cooldown_period_seconds;
        let now = self.clock.unix_now();
        let result = self
            .with_store("claim_next_due", move |store| {
                store.claim_next_due(cooldown, now)
            })
            .await;
        if matches!(&result, Ok(Some((_, 0)))) {
            self.update_queue_gauges().await;
        }
        result
    }

    /// Reconcile interrupted completion and release claims from a prior worker.
    pub(crate) async fn recover_worker_state(&self) -> StoreResult<()> {
        self.with_store("recover_worker_state", QueueStore::recover_worker_state)
            .await?;
        self.update_queue_gauges().await;
        Ok(())
    }

    /// Remove the posted entry and record the posting time.
    pub async fn complete(&self, id: &str, claim_token: Option<String>) -> StoreResult<()> {
        let id = id.to_owned();
        let now = self.clock.unix_now();
        let result = self
            .with_store("complete", move |store| {
                store.complete_claim(&id, claim_token.as_deref(), now)
            })
            .await;
        if result.is_ok() {
            self.update_queue_gauges().await;
        }
        result
    }

    /// Release a failed post's claim before waiting for its retry deadline.
    pub async fn release_claim(&self, id: &str, claim_token: &str) -> StoreResult<()> {
        let id = id.to_owned();
        let claim_token = claim_token.to_owned();
        let result = self
            .with_store("release_claim", move |store| {
                store.release_claim(&id, &claim_token)
            })
            .await;
        if result.is_ok() {
            self.update_queue_gauges().await;
        }
        result
    }

    /// Persist a put request and return its schedule-stable queue entry.
    pub(crate) async fn put(
        &self,
        request: CommentRequest,
        immediate: bool,
    ) -> StoreResult<(StoredEntry, u64)> {
        validate_request(&request)?;
        let cooldown = self.cfg.cooldown_period_seconds;
        let flutter_max = self.cfg.cooldown_flutter_seconds;
        let flutter_seconds = self.flutter_sampler.sample(flutter_max).min(flutter_max);
        let now = self.clock.unix_now();
        let result = self
            .with_store("put", move |store| {
                let options = PutOptions {
                    cooldown,
                    flutter_seconds,
                    immediate,
                };
                store.put_with_eta(request, &options, now)
            })
            .await;
        if result.is_ok() {
            self.mutation_finished().await;
        }
        result
    }

    /// Return the ordered schedule without constructing a protocol response.
    pub(crate) async fn list(&self) -> StoreResult<Vec<(StoredEntry, u64)>> {
        let cooldown = self.cfg.cooldown_period_seconds;
        let now = self.clock.unix_now();
        self.with_store("list", move |store| store.schedule(cooldown, now))
            .await
    }

    /// Move the identified entry to the head of the queue.
    pub(crate) async fn bump(&self, id: &str) -> StoreResult<()> {
        self.mutate_entry("bump", id, QueueStore::bump).await
    }

    /// Move the identified entry to the tail of the queue.
    pub(crate) async fn bust(&self, id: &str) -> StoreResult<()> {
        self.mutate_entry("bust", id, QueueStore::bust).await
    }

    /// Remove the identified entry from the queue.
    pub(crate) async fn del(&self, id: &str) -> StoreResult<()> {
        self.mutate_entry("del", id, QueueStore::del).await
    }

    async fn mutate_entry(
        &self,
        operation: &'static str,
        id: &str,
        mutate: fn(&QueueStore, &str) -> StoreResult<()>,
    ) -> StoreResult<()> {
        let id = id.to_owned();
        let result = self
            .with_store(operation, move |store| mutate(store, &id))
            .await;
        if result.is_ok() {
            self.mutation_finished().await;
        }
        result
    }

    async fn mutation_finished(&self) {
        self.update_queue_gauges().await;
        // notify_one buffers a permit while the worker is processing an entry.
        self.changed.notify_one();
    }

    /// Run synchronous store work outside Tokio's asynchronous executor.
    async fn with_store<T, F>(&self, operation: &'static str, work: F) -> StoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&QueueStore) -> StoreResult<T> + Send + 'static,
    {
        let started = Instant::now();
        let store = Arc::clone(&self.store);
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
            let store = store
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            work(&store)
        })
        .await
        .unwrap_or_else(|error| Err(StoreError::BlockingTask(error)));
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
    async fn update_queue_gauges(&self) {
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

/// Reject repository components that could alter paths, URLs, or terminal output.
fn validate_request(request: &CommentRequest) -> StoreResult<()> {
    if !is_safe_repository_component(&request.owner) {
        return Err(crate::store::StoreError::InvalidRepositoryComponent(
            "owner",
        ));
    }
    if !is_safe_repository_component(&request.repo) {
        return Err(crate::store::StoreError::InvalidRepositoryComponent("name"));
    }
    Ok(())
}

/// Report whether a GitHub owner or repository name is safe at this boundary.
fn is_safe_repository_component(component: &str) -> bool {
    !component.is_empty()
        && component.chars().all(|character| {
            !character.is_control()
                && !matches!(character, '/' | '\\' | '?' | '#' | '\u{2028}' | '\u{2029}')
        })
}

#[cfg(test)]
mod tests;
