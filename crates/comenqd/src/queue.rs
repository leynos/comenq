//! Shared queue state and protocol operation dispatch.
//!
//! [`SharedQueue`] bundles the persistent [`QueueStore`] with the daemon
//! configuration and a change signal. The listener executes protocol
//! requests against it, and the worker waits on the change signal so queue
//! mutations (put, bump, bust, del) are observed promptly.

use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use comenq_lib::CommentRequest;
use comenq_lib::protocol::{MAX_PENDING_ENTRIES, MAX_RESPONSE_BYTES, Request, Response};
use rand::Rng;
use tokio::sync::Notify;

use crate::config::Config;
use crate::metrics;
use crate::store::{PutOptions, QueueStore, Result as StoreResult, StoredEntry};

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

trait FlutterSampler: Debug + Send + Sync {
    fn sample(&self, maximum: u64) -> u64;
}

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
        Self::open_with_clock_and_flutter(cfg, clock, Arc::new(RandomFlutterSampler))
    }

    /// Open a queue with explicit wall-clock and enqueue-flutter sources.
    fn open_with_clock_and_flutter(
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
        self.with_store(move |store| store.next_due(cooldown, now))
            .await
    }

    /// Claim the due entry while holding the store lock.
    pub async fn claim_next_due(&self) -> StoreResult<Option<(StoredEntry, u64)>> {
        let cooldown = self.cfg.cooldown_period_seconds;
        let now = self.clock.unix_now();
        let result = self
            .with_store(move |store| store.claim_next_due(cooldown, now))
            .await;
        if matches!(&result, Ok(Some((_, 0)))) {
            self.update_queue_gauges().await;
        }
        result
    }

    /// Reconcile interrupted completion and release claims from a prior worker.
    pub(crate) async fn recover_worker_state(&self) -> StoreResult<()> {
        self.with_store(QueueStore::recover_worker_state).await?;
        self.update_queue_gauges().await;
        Ok(())
    }

    /// Remove the posted entry and record the posting time.
    pub async fn complete(&self, id: &str, claim_token: Option<String>) -> StoreResult<()> {
        let id = id.to_owned();
        let now = self.clock.unix_now();
        let result = self
            .with_store(move |store| store.complete_claim(&id, claim_token.as_deref(), now))
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
            .with_store(move |store| store.release_claim(&id, &claim_token))
            .await;
        if result.is_ok() {
            self.update_queue_gauges().await;
        }
        result
    }

    /// Execute a protocol request and produce the reply.
    ///
    /// Mutations signal the worker through the change notifier. Failures are
    /// reported to the client as [`Response::Error`]; they never propagate.
    pub async fn execute(&self, request: Request) -> Response {
        let (response, mutated) = match request {
            Request::Put { request, immediate } => {
                (self.execute_put(request, immediate).await, true)
            }
            Request::List => (self.execute_list().await, false),
            Request::Bump { id } => (
                self.with_store(move |store| store.bump(&id).map(|()| Response::ok()))
                    .await,
                true,
            ),
            Request::Bust { id } => (
                self.with_store(move |store| store.bust(&id).map(|()| Response::ok()))
                    .await,
                true,
            ),
            Request::Del { id } => (
                self.with_store(move |store| store.del(&id).map(|()| Response::ok()))
                    .await,
                true,
            ),
        };
        match response {
            Ok(reply) => {
                if mutated {
                    self.update_queue_gauges().await;
                    // notify_one buffers a permit, so a worker that is busy
                    // computing rather than parked still observes the change.
                    self.changed.notify_one();
                }
                reply
            }
            Err(e) => Response::error(e.to_string()),
        }
    }

    /// Persist a put request and return its schedule-stable pending entry.
    async fn execute_put(&self, request: CommentRequest, immediate: bool) -> StoreResult<Response> {
        validate_request(&request)?;
        let cooldown = self.cfg.cooldown_period_seconds;
        let flutter_max = self.cfg.cooldown_flutter_seconds;
        let flutter_seconds = self.flutter_sampler.sample(flutter_max).min(flutter_max);
        let now = self.clock.unix_now();
        self.with_store(move |store| {
            let options = PutOptions {
                cooldown,
                flutter_seconds,
                immediate,
            };
            store
                .put_with_eta(request, &options, now)
                .map(|(entry, eta)| Response::entry(entry.to_pending(eta)))
        })
        .await
    }

    /// Convert the current persisted schedule into a list response.
    async fn execute_list(&self) -> StoreResult<Response> {
        let cooldown = self.cfg.cooldown_period_seconds;
        let now = self.clock.unix_now();
        self.with_store(move |store| {
            store
                .schedule(cooldown, now)
                .and_then(response_for_schedule)
        })
        .await
    }

    /// Run synchronous store work outside Tokio's asynchronous executor.
    async fn with_store<T, F>(&self, operation: F) -> StoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&QueueStore) -> StoreResult<T> + Send + 'static,
    {
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || {
            let store = store
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            operation(&store)
        })
        .await?
    }

    async fn update_queue_gauges(&self) {
        match self.with_store(QueueStore::queue_metrics_snapshot).await {
            Ok((count, bytes)) => {
                metrics::record_queue_entries(count);
                metrics::record_queue_bytes(bytes);
            }
            Err(error) => tracing::warn!(error = %error, "Failed to refresh queue metrics"),
        }
    }
}

fn response_for_schedule(schedule: Vec<(StoredEntry, u64)>) -> StoreResult<Response> {
    let mut projected_size = serde_json::to_vec(&Response::entries(Vec::new()))?.len();
    let mut entries = Vec::with_capacity(schedule.len().min(MAX_PENDING_ENTRIES));

    for (entry, eta) in schedule.into_iter().take(MAX_PENDING_ENTRIES) {
        let pending = entry.to_pending(eta);
        let pending_size = serde_json::to_vec(&pending)?.len();
        let separator_size = usize::from(!entries.is_empty());
        let next_size = projected_size
            .saturating_add(pending_size)
            .saturating_add(separator_size);
        if next_size > MAX_RESPONSE_BYTES {
            return Ok(Response::error(format!(
                "list response exceeds the {MAX_RESPONSE_BYTES}-byte limit"
            )));
        }
        projected_size = next_size;
        entries.push(pending);
    }

    Ok(Response::entries(entries))
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
