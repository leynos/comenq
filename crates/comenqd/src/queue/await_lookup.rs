//! Read-only posting outcome queries serialized with queue mutations.

use super::{SharedQueue, store_access::QueueState};
use crate::store::{AwaitCursor, AwaitState, QueueStore, Result, StoreError};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Opaque continuation token for one selected queue-entry lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitToken(AwaitCursor);

/// Result of resolving an identifier through the queue use case.
#[derive(Debug, PartialEq, Eq)]
pub enum AwaitOutcome {
    /// The entry remains queued for the indicated time.
    Pending {
        /// Estimated seconds until the entry is due.
        eta_seconds: u64,
        /// Opaque token that keeps later queries bound to this entry lifetime.
        token: AwaitToken,
    },
    /// A successful posting was recorded.
    Posted {
        /// Full GitHub comment URL, absent in legacy history.
        comment_url: Option<String>,
        /// Unix time of the successful posting attempt.
        posted_at: u64,
    },
    /// No pending entry or completion is known for the identifier.
    Unknown,
}

/// Bounded error returned by the await use case.
#[derive(Debug, thiserror::Error)]
pub enum AwaitError {
    /// The identifier does not have the required eight hexadecimal characters.
    #[error("await identifier must contain eight hexadecimal characters")]
    InvalidIdentifier,
    /// The queue could not resolve the request; the category is a fixed label.
    #[error("await lookup failed ({0})")]
    QueueLookup(&'static str),
}

impl AwaitError {
    /// Return the bounded label suitable for metrics and structured logs.
    pub(crate) fn category(&self) -> &'static str {
        match self {
            Self::InvalidIdentifier => "invalid_identifier",
            Self::QueueLookup(category) => category,
        }
    }

    /// Return a user-facing message without including caller-controlled input.
    pub(crate) fn message(&self) -> String {
        self.to_string()
    }
}

impl From<AwaitState> for AwaitOutcome {
    fn from(state: AwaitState) -> Self {
        match state {
            AwaitState::Pending {
                eta_seconds,
                cursor,
            } => Self::Pending {
                eta_seconds,
                token: AwaitToken(cursor),
            },
            AwaitState::Posted {
                comment_url,
                posted_at,
            } => Self::Posted {
                comment_url,
                posted_at,
            },
            AwaitState::Unknown => Self::Unknown,
        }
    }
}

/// Application-level view of queue changes, independent of Tokio's watch API.
pub(crate) struct StateSubscription(tokio::sync::watch::Receiver<u64>);

impl StateSubscription {
    /// Mark the current queue generation as observed before the first lookup.
    pub(crate) fn mark_seen(&mut self) {
        self.0.borrow_and_update();
    }

    /// Wait until the queue publishes a new generation.
    pub(crate) async fn changed(&mut self) {
        let _ = self.0.changed().await;
    }
}

/// One projection shared by all waiters for a state version and clock second.
pub(super) struct AwaitSchedule {
    built_at: u64,
    etas: HashMap<String, u64>,
}

struct AwaitSnapshot {
    generation: u64,
    pending: Option<AwaitState>,
    store: QueueStore,
}

impl SharedQueue {
    /// Subscribe to queue-state versions without consuming worker wakeups.
    pub(crate) fn subscribe_state(&self) -> StateSubscription {
        StateSubscription(self.state_version.subscribe())
    }

    /// Resolve an identifier against the queue and its durable completion history.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use comenqd::queue::SharedQueue;
    /// # async fn query(queue: &SharedQueue) -> Result<(), comenqd::queue::AwaitError> {
    /// let outcome = queue.resolve_await("1a2b3c4d", None).await?;
    /// println!("{outcome:?}");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resolve_await(
        &self,
        id: &str,
        token: Option<AwaitToken>,
    ) -> std::result::Result<AwaitOutcome, AwaitError> {
        let cursor = token.map(|token| token.0);
        self.resolve_await_store(id, cursor)
            .await
            .map(AwaitOutcome::from)
            .map_err(|error| match error {
                StoreError::InvalidId(_) => AwaitError::InvalidIdentifier,
                other => AwaitError::QueueLookup(other.category()),
            })
    }

    /// Run the persistence adapter's lookup while keeping its cursor private.
    async fn resolve_await_store(
        &self,
        id: &str,
        cursor: Option<AwaitCursor>,
    ) -> Result<AwaitState> {
        self.resolve_await_store_with_history(id, cursor, |store, id, cursor| {
            store.resolve_await_completion(&id, cursor.as_ref())
        })
        .await
    }

    /// Keep the history adapter replaceable for deterministic concurrency tests.
    async fn resolve_await_store_with_history<F>(
        &self,
        id: &str,
        cursor: Option<AwaitCursor>,
        history_lookup: F,
    ) -> Result<AwaitState>
    where
        F: Fn(crate::store::QueueStore, String, Option<AwaitCursor>) -> Result<AwaitState>
            + Clone
            + Send
            + Sync
            + 'static,
    {
        let id = id.to_owned();
        let cooldown = self.cfg.cooldown_period_seconds;
        loop {
            let observed = self
                .observe_await_state(id.clone(), cursor.clone(), cooldown)
                .await?;
            if let Some(pending) = observed.pending {
                return Ok(pending);
            }

            let completed = scan_await_history(
                observed.store,
                id.clone(),
                cursor.clone(),
                history_lookup.clone(),
            )
            .await;

            // Recheck pending state and the generation after the unlocked scan.
            // A concurrent put/completion invalidates the history snapshot.
            let current = self
                .observe_await_state(id.clone(), cursor.clone(), cooldown)
                .await?;
            if let Some(pending) = current.pending {
                return Ok(pending);
            }
            if current.generation == observed.generation {
                return completed;
            }
        }
    }

    async fn observe_await_state(
        &self,
        id: String,
        cursor: Option<AwaitCursor>,
        cooldown: u64,
    ) -> Result<AwaitSnapshot> {
        let now = self.clock.unix_now();
        let state_version = Arc::clone(&self.state_version);
        self.with_state("resolve_await", move |state| {
            let generation = *state_version.borrow();
            let pending = resolve_pending(state, &id, cursor.as_ref(), cooldown, now)?;
            Ok(AwaitSnapshot {
                generation,
                pending,
                store: state.store.clone(),
            })
        })
        .await
    }
}

async fn scan_await_history<F>(
    store: QueueStore,
    id: String,
    cursor: Option<AwaitCursor>,
    history_lookup: F,
) -> Result<AwaitState>
where
    F: Fn(QueueStore, String, Option<AwaitCursor>) -> Result<AwaitState> + Send + Sync + 'static,
{
    let history_started = Instant::now();
    let history_result = tokio::task::spawn_blocking(move || history_lookup(store, id, cursor))
        .await
        .unwrap_or_else(|error| Err(StoreError::BlockingTask(error)));
    let duration = history_started.elapsed();
    let (outcome, error_kind) = match &history_result {
        Ok(_) => ("success", None),
        Err(error) => ("failure", Some(error.category())),
    };
    crate::metrics::record_queue_store_duration("resolve_await", outcome, error_kind, duration);
    if let Err(error) = &history_result
        && error.is_unexpected()
    {
        tracing::warn!(
            operation = "resolve_await",
            error_kind = error.category(),
            "Queue history lookup failed"
        );
    }
    history_result
}

/// Resolve only the selected pending lifetime while the shared queue is locked.
fn resolve_pending(
    state: &mut QueueState,
    id: &str,
    cursor: Option<&AwaitCursor>,
    cooldown: u64,
    now: u64,
) -> Result<Option<AwaitState>> {
    #[cfg(test)]
    let needs_projection = state
        .await_schedule
        .as_ref()
        .is_none_or(|cached| cached.built_at != now);
    let outcome = state
        .store
        .resolve_await_pending_with_eta(id, cursor, |store, entry| {
            cached_eta(store, &mut state.await_schedule, &entry.id, cooldown, now)
        });
    #[cfg(test)]
    if needs_projection
        && state
            .await_schedule
            .as_ref()
            .is_some_and(|cached| cached.built_at == now)
    {
        state.projection_builds += 1;
    }
    outcome
}

/// Operations that can alter an await lookup or its projected schedule.
pub(super) fn is_state_mutation(operation: &str) -> bool {
    matches!(
        operation,
        "put"
            | "bump"
            | "bust"
            | "del"
            | "claim_next_due"
            | "complete"
            | "release_claim"
            | "recover_worker_state"
    )
}

/// Refresh at most once per version and clock second; mutations invalidate this cache.
fn cached_eta(
    store: &crate::store::QueueStore,
    cache: &mut Option<AwaitSchedule>,
    id: &str,
    cooldown: u64,
    now: u64,
) -> Result<u64> {
    let should_rebuild = cache
        .as_ref()
        .is_none_or(|projection| projection.built_at != now);
    if should_rebuild {
        crate::metrics::record_await_schedule_cache("rebuild");
        *cache = Some(AwaitSchedule {
            built_at: now,
            etas: store
                .schedule(cooldown, now)?
                .into_iter()
                .map(|(entry, eta)| (entry.id, eta))
                .collect(),
        });
    } else {
        crate::metrics::record_await_schedule_cache("hit");
    }
    Ok(cache
        .as_ref()
        .and_then(|projection| projection.etas.get(id))
        .copied()
        .unwrap_or(0))
}

#[cfg(test)]
mod tests;
