//! Shared queue state and protocol operation dispatch.
//!
//! [`SharedQueue`] bundles the persistent [`QueueStore`] with the daemon
//! configuration and a change signal. The listener executes protocol
//! requests against it, and the worker waits on the change signal so queue
//! mutations (put, bump, bust, del) are observed promptly.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use comenq_lib::protocol::{Request, Response};
use tokio::sync::{Mutex, Notify};

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

/// Queue state shared between the listener and the worker.
#[derive(Debug)]
pub struct SharedQueue {
    cfg: Arc<Config>,
    store: Mutex<QueueStore>,
    changed: Notify,
}

impl SharedQueue {
    /// Open the queue store described by `cfg`.
    pub fn open(cfg: Arc<Config>) -> StoreResult<Arc<Self>> {
        let store = QueueStore::open(&cfg.queue_path)?;
        Ok(Arc::new(Self {
            cfg,
            store: Mutex::new(store),
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
        self.store
            .lock()
            .await
            .next_due(self.cfg.cooldown_period_seconds, unix_now())
    }

    /// Remove the posted entry and record the posting time.
    pub async fn complete(&self, id: &str) -> StoreResult<()> {
        self.store.lock().await.complete(id, unix_now())
    }

    /// Execute a protocol request and produce the reply.
    ///
    /// Mutations signal the worker through the change notifier. Failures are
    /// reported to the client as [`Response::Error`]; they never propagate.
    pub async fn execute(&self, request: Request) -> Response {
        let store = self.store.lock().await;
        let cooldown = self.cfg.cooldown_period_seconds;
        let now = unix_now();
        let (response, mutated) = match request {
            Request::Put { request, immediate } => {
                let options = PutOptions {
                    cooldown,
                    flutter_max: self.cfg.cooldown_flutter_seconds,
                    immediate,
                };
                let outcome = store.put(request, &options, now).and_then(|entry| {
                    let eta = store
                        .schedule(cooldown, now)?
                        .into_iter()
                        .find(|(scheduled, _)| scheduled.id == entry.id)
                        .map_or(0, |(_, eta)| eta);
                    Ok(Response::entry(entry.to_pending(eta)))
                });
                (outcome, true)
            }
            Request::List => {
                let outcome = store.schedule(cooldown, now).map(|schedule| {
                    Response::entries(
                        schedule
                            .into_iter()
                            .map(|(entry, eta)| entry.to_pending(eta))
                            .collect(),
                    )
                });
                (outcome, false)
            }
            Request::Bump { id } => (store.bump(&id).map(|()| Response::ok()), true),
            Request::Bust { id } => (store.bust(&id).map(|()| Response::ok()), true),
            Request::Del { id } => (store.del(&id).map(|()| Response::ok()), true),
        };
        drop(store);
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
}
