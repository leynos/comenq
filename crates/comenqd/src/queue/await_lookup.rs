//! Read-only posting outcome queries serialized with queue mutations.

use super::{SharedQueue, store_access::QueueState};
use crate::store::{AwaitCursor, AwaitState, Result};
use std::collections::HashMap;

/// One projection shared by all waiters for a state version and clock second.
pub(super) struct AwaitSchedule {
    built_at: u64,
    etas: HashMap<String, u64>,
}

impl SharedQueue {
    /// Subscribe to queue-state versions without consuming worker wakeups.
    pub(crate) fn subscribe_state(&self) -> tokio::sync::watch::Receiver<u64> {
        self.state_version.subscribe()
    }

    /// Resolve an identifier against the queue and its durable completion history.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use comenqd::queue::SharedQueue;
    /// # async fn query(queue: &SharedQueue) -> Result<(), comenqd::store::StoreError> {
    /// let outcome = queue.resolve_await("1a2b3c4d", None).await?;
    /// println!("{outcome:?}");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resolve_await(&self, id: &str, cursor: Option<AwaitCursor>) -> Result<AwaitState> {
        let id = id.to_owned();
        let cooldown = self.cfg.cooldown_period_seconds;
        let now = self.clock.unix_now();
        self.with_state("resolve_await", move |state| {
            #[cfg(test)]
            let needs_projection = state
                .await_schedule
                .as_ref()
                .is_none_or(|cached| cached.built_at != now);
            let QueueState {
                store,
                await_schedule,
                ..
            } = state;
            let outcome = store.resolve_await_with_eta(&id, cursor.as_ref(), |store, entry| {
                cached_eta(store, await_schedule, &entry.id, cooldown, now)
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
        })
        .await
    }
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
    if cache
        .as_ref()
        .is_none_or(|projection| projection.built_at != now)
    {
        *cache = Some(AwaitSchedule {
            built_at: now,
            etas: store
                .schedule(cooldown, now)?
                .into_iter()
                .map(|(entry, eta)| (entry.id, eta))
                .collect(),
        });
    }
    Ok(cache
        .as_ref()
        .and_then(|projection| projection.etas.get(id))
        .copied()
        .unwrap_or(0))
}

#[cfg(test)]
mod tests;
