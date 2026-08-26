//! Shared queue state and protocol operation dispatch.
//!
//! [`SharedQueue`] bundles the persistent [`QueueStore`] with the daemon
//! configuration and a change signal. The listener executes protocol
//! requests against it, and the worker waits on the change signal so queue
//! mutations (put, bump, bust, del) are observed promptly.

use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use comenq_lib::protocol::{Request, Response};
use tokio::sync::Notify;

use crate::config::Config;
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
    changed: Notify,
}

impl SharedQueue {
    /// Open the queue store described by `cfg`.
    pub fn open(cfg: Arc<Config>) -> StoreResult<Arc<Self>> {
        Self::open_with_clock(cfg, Arc::new(SystemClock))
    }

    /// Open the queue store using `clock` for persisted scheduling timestamps.
    pub fn open_with_clock(cfg: Arc<Config>, clock: Arc<dyn UnixClock>) -> StoreResult<Arc<Self>> {
        let store = QueueStore::open(&cfg.queue_path)?;
        Ok(Arc::new(Self {
            cfg,
            store: Arc::new(Mutex::new(store)),
            clock,
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

    /// The head entry and its estimated seconds-until-post, when any.
    pub async fn next_due(&self) -> StoreResult<Option<(StoredEntry, u64)>> {
        let cooldown = self.cfg.cooldown_period_seconds;
        let now = self.clock.unix_now();
        self.with_store(move |store| store.next_due(cooldown, now))
            .await
    }

    /// Remove the posted entry and record the posting time.
    pub async fn complete(&self, id: &str) -> StoreResult<()> {
        let id = id.to_owned();
        let now = self.clock.unix_now();
        self.with_store(move |store| store.complete(&id, now)).await
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
                    // notify_one buffers a permit, so a worker that is busy
                    // computing rather than parked still observes the change.
                    self.changed.notify_one();
                }
                reply
            }
            Err(e) => Response::error(e.to_string()),
        }
    }

    async fn execute_put(
        &self,
        request: comenq_lib::CommentRequest,
        immediate: bool,
    ) -> StoreResult<Response> {
        let cooldown = self.cfg.cooldown_period_seconds;
        let flutter_max = self.cfg.cooldown_flutter_seconds;
        let now = self.clock.unix_now();
        self.with_store(move |store| {
            let options = PutOptions {
                cooldown,
                flutter_max,
                immediate,
            };
            store.put(request, &options, now).and_then(|entry| {
                let eta = store
                    .schedule(cooldown, now)?
                    .into_iter()
                    .find(|(scheduled, _)| scheduled.id == entry.id)
                    .map_or(0, |(_, eta)| eta);
                Ok(Response::entry(entry.to_pending(eta)))
            })
        })
        .await
    }

    async fn execute_list(&self) -> StoreResult<Response> {
        let cooldown = self.cfg.cooldown_period_seconds;
        let now = self.clock.unix_now();
        self.with_store(move |store| {
            store.schedule(cooldown, now).map(|schedule| {
                Response::entries(
                    schedule
                        .into_iter()
                        .map(|(entry, eta)| entry.to_pending(eta))
                        .collect(),
                )
            })
        })
        .await
    }

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
}

#[cfg(test)]
mod tests {
    //! Deterministic scheduling tests for the shared queue clock boundary.

    use super::{SharedQueue, UnixClock};
    use crate::config::Config;
    use comenq_lib::CommentRequest;
    use comenq_lib::protocol::{Request, Response};
    use std::sync::Arc;
    use tempfile::tempdir;

    #[derive(Debug)]
    struct FixedClock(u64);

    impl UnixClock for FixedClock {
        fn unix_now(&self) -> u64 {
            self.0
        }
    }

    #[tokio::test]
    async fn fixed_clock_controls_deferred_put_eta() {
        let dir = tempdir().expect("create temporary queue directory");
        let queue = SharedQueue::open_with_clock(
            Arc::new(Config {
                github_token: "token".into(),
                github_token_file: None,
                socket_path: dir.path().join("comenq.sock"),
                queue_path: dir.path().join("queue"),
                cooldown_period_seconds: 600,
                cooldown_flutter_seconds: 0,
                restart_min_delay_ms: 1,
                github_api_timeout_secs: 1,
            }),
            Arc::new(FixedClock(1_000)),
        )
        .expect("open queue");

        let response = queue
            .execute(Request::Put {
                request: CommentRequest {
                    owner: "octocat".into(),
                    repo: "hello-world".into(),
                    pr_number: 7,
                    body: "comment".into(),
                },
                immediate: false,
            })
            .await;
        let Response::Ok {
            entry: Some(entry), ..
        } = response
        else {
            panic!("expected queued entry, got {response:?}");
        };
        assert_eq!(entry.eta_seconds, 600);
    }
}
