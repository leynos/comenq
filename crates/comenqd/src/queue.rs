//! Shared queue state and queue use-case operations.
//!
//! [`SharedQueue`] bundles the persistent [`QueueStore`] with the daemon
//! configuration and a change signal. Each operation composes a short-lived
//! [`QueueService`] over the borrowed store, so queue policy stays in the
//! domain while persistence, time and randomness remain adapter concerns.
//!
//! The listener maps protocol operations to these typed operations, and the
//! worker waits on the change signal so queue mutations are observed promptly.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use comenq_domain::{
    EntryId, PutComment, QueueError, QueuePolicy, QueueService, QueuedEntry, ScheduledEntry,
};
use rand::Rng;
use tokio::sync::{Notify, watch};

use crate::config::Config;
use crate::metrics;
use crate::store::{
    QueueStore, Result as StoreResult, StoreError, StoredEntry, store_error_from_queue,
};

use self::convert::{queued_from_stored, stored_schedule};

/// A queue operation result that keeps domain and storage failures distinct.
pub(crate) type QueueResult<T> = Result<T, QueueError<StoreError>>;

/// Wall clock used for timestamps persisted in the queue store.
pub use comenq_domain::WallClock as UnixClock;

/// Provide the bounded random delay added to a newly enqueued comment.
pub(crate) use comenq_domain::FlutterSampler;

/// Return a bounded classification for queue failures without exposing values.
pub(crate) fn queue_error_category(error: &QueueError<StoreError>) -> &'static str {
    match error {
        QueueError::UnknownIdentifier(_) => "unknown_identifier",
        QueueError::InvalidIdentifier(_) => "unsafe_identifier",
        QueueError::InvalidRepositoryComponent(_) => "invalid_repository_component",
        QueueError::Repository(error) => error.category(),
    }
}

/// Current Unix time in whole seconds.
///
/// Clamps to zero should the system clock report a time before the epoch.
#[must_use]
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
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
    store: Arc<Mutex<QueueState>>,
    clock: Arc<dyn UnixClock>,
    flutter_sampler: Arc<dyn FlutterSampler>,
    policy: QueuePolicy,
    changed: Notify,
    state_version: Arc<watch::Sender<u64>>,
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
        let policy = QueuePolicy::new(cfg.cooldown_period_seconds, cfg.cooldown_flutter_seconds);
        let store = QueueStore::open(&cfg.queue_path)?;
        let (entry_count, accounted_bytes) = store.queue_metrics_snapshot()?;
        metrics::record_queue_entries(entry_count);
        metrics::record_queue_bytes(accounted_bytes);
        Ok(Arc::new(Self {
            cfg,
            store: Arc::new(Mutex::new(QueueState::new(store))),
            clock,
            flutter_sampler,
            policy,
            changed: Notify::new(),
            state_version: Arc::new(watch::channel(0).0),
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
    ///
    /// This compatibility view returns the stored layout. Domain-aware callers
    /// use [`Self::next_due_domain`].
    pub async fn next_due(&self) -> StoreResult<Option<(StoredEntry, u64)>> {
        self.next_due_domain()
            .await
            .map_err(store_error_from_queue)
            .map(|due| due.map(stored_schedule))
    }

    /// The head entry as a domain value, when nothing is claimed.
    pub(crate) async fn next_due_domain(&self) -> QueueResult<Option<ScheduledEntry>> {
        self.with_service("next_due", |store, clock, sampler, policy| {
            service(store, clock, sampler, policy).next_due()
        })
        .await
    }

    /// Claim the due entry while holding the store lock.
    ///
    /// This compatibility view returns the stored layout. The worker uses
    /// [`Self::claim_next_due_domain`].
    pub async fn claim_next_due(&self) -> StoreResult<Option<(StoredEntry, u64)>> {
        self.claim_next_due_domain()
            .await
            .map_err(store_error_from_queue)
            .map(|due| due.map(stored_schedule))
    }

    /// Claim the due entry as a domain value while holding the store lock.
    pub(crate) async fn claim_next_due_domain(&self) -> QueueResult<Option<ScheduledEntry>> {
        let result = self
            .with_service("claim_next_due", |store, clock, sampler, policy| {
                service(store, clock, sampler, policy).claim_next_due()
            })
            .await;
        if matches!(&result, Ok(Some(scheduled)) if scheduled.eta_seconds == 0) {
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

    /// Durably record a successful post before removing its queue entry.
    ///
    /// This compatibility view accepts the stored layout. Domain-aware callers
    /// use [`Self::complete_entry`].
    pub async fn complete(
        &self,
        entry: &StoredEntry,
        token_hash: &str,
        comment_url: &str,
    ) -> StoreResult<()> {
        let entry = queued_from_stored(entry.clone())?;
        self.complete_entry(&entry, token_hash, Some(comment_url.to_owned()))
            .await
            .map_err(store_error_from_queue)
    }

    /// Durably record a successful post for a claimed domain entry.
    pub(crate) async fn complete_entry(
        &self,
        entry: &QueuedEntry,
        token_hash: &str,
        comment_url: Option<String>,
    ) -> QueueResult<()> {
        let entry = entry.clone();
        let token_hash = token_hash.to_owned();
        let result = self
            .with_service("complete", move |store, clock, sampler, policy| {
                service(store, clock, sampler, policy).complete(entry, token_hash, comment_url)
            })
            .await;
        if result.is_ok() {
            self.update_queue_gauges().await;
        }
        result
    }

    /// Release a failed post's claim before waiting for its retry deadline.
    ///
    /// A stale claim is treated as success because another actor already
    /// cleared or replaced it.
    pub async fn release_claim(&self, id: &str, claim_token: &str) -> StoreResult<()> {
        let id = id.to_owned();
        let claim_token = claim_token.to_owned();
        let result = self
            .with_service("release_claim", move |store, clock, sampler, policy| {
                service(store, clock, sampler, policy).release_claim(&id, &claim_token)
            })
            .await;
        if result.is_ok() {
            self.update_queue_gauges().await;
        }
        result.map(|_| ()).map_err(store_error_from_queue)
    }

    /// Persist a put request and return its schedule-stable queue entry.
    pub(crate) async fn put(&self, request: PutComment) -> QueueResult<ScheduledEntry> {
        let result = self
            .with_service("put", move |store, clock, sampler, policy| {
                service(store, clock, sampler, policy).put(request)
            })
            .await;
        if result.is_ok() {
            self.mutation_finished().await;
        }
        result
    }

    /// Return the ordered schedule without constructing a protocol response.
    pub(crate) async fn list(&self) -> QueueResult<Vec<ScheduledEntry>> {
        self.with_service("list", |store, clock, sampler, policy| {
            service(store, clock, sampler, policy).schedule()
        })
        .await
    }

    /// Move the identified entry to the head of the queue.
    pub(crate) async fn bump(&self, id: &str) -> QueueResult<()> {
        let id = id.to_owned();
        let result = self
            .with_service("bump", move |store, clock, sampler, policy| {
                service(store, clock, sampler, policy).bump(&id)
            })
            .await;
        if result.is_ok() {
            self.mutation_finished().await;
        }
        result
    }

    /// Move the identified entry to the tail of the queue.
    pub(crate) async fn bust(&self, id: &str) -> QueueResult<()> {
        let id = id.to_owned();
        let result = self
            .with_service("bust", move |store, clock, sampler, policy| {
                service(store, clock, sampler, policy).bust(&id)
            })
            .await;
        if result.is_ok() {
            self.mutation_finished().await;
        }
        result
    }

    /// Remove the identified entry from the queue.
    pub(crate) async fn del(&self, id: &str) -> QueueResult<()> {
        let id = id.to_owned();
        let result = self
            .with_service("del", move |store, clock, sampler, policy| {
                service(store, clock, sampler, policy).del(&id)
            })
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
}

/// Compose a domain service from the queue's ports and posting policy.
fn service<'a>(
    store: &'a QueueStore,
    clock: &'a dyn UnixClock,
    sampler: &'a dyn FlutterSampler,
    policy: QueuePolicy,
) -> QueueService<&'a QueueStore, &'a dyn UnixClock, &'a dyn FlutterSampler> {
    QueueService::new(store, clock, sampler, policy)
}

/// Parse an identifier through the domain's validation rule.
pub(crate) fn parse_entry_id(id: &str) -> Result<EntryId, QueueError<StoreError>> {
    EntryId::parse(id).map_err(QueueError::InvalidIdentifier)
}

#[cfg(test)]
mod tests;

mod await_lookup;
pub(crate) mod convert;
pub use await_lookup::{AwaitError, AwaitOutcome, AwaitToken};
mod store_access;
use store_access::QueueState;
mod history;
