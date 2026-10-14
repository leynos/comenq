//! Read-only posting outcome queries serialized with queue mutations.

use super::SharedQueue;
use crate::store::{AwaitCursor, AwaitState, Result};

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
        self.with_store("resolve_await", move |store| {
            store.resolve_await(&id, cursor.as_ref(), cooldown, now)
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
