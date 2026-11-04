//! Posting-history operations on shared queue state.

use comenq_domain::QueuedEntry;

use crate::store::{
    HistoryPage, HistoryPosition, HistoryRecord, Result as StoreResult, StoredEntry,
};

use super::SharedQueue;
use super::convert::stored_from_queued;

/// Maximum records loaded for one listener history page before byte sizing.
const HISTORY_READ_PAGE_SIZE: usize = 64;

impl SharedQueue {
    /// Append a failed attempt while keeping the entry queued for a retry.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use comenqd::daemon::SharedQueue;
    /// # use comenqd::store::StoredEntry;
    /// # async fn record(queue: &SharedQueue, entry: &StoredEntry) -> Result<(), Box<dyn std::error::Error>> {
    /// queue.record_failure(entry, "token-hash", "request timed out").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn record_failure(
        &self,
        entry: &StoredEntry,
        token_hash: &str,
        error: &str,
    ) -> StoreResult<()> {
        self.append_history(HistoryRecord::failure(
            entry,
            token_hash,
            self.clock.unix_now(),
            error,
        ))
        .await
    }

    /// Append a failed attempt for a domain entry while keeping it queued.
    pub(crate) async fn record_failure_for(
        &self,
        entry: &QueuedEntry,
        token_hash: &str,
        error: &str,
    ) -> StoreResult<()> {
        let entry = stored_from_queued(entry);
        self.record_failure(&entry, token_hash, error).await
    }

    /// Hash of the token used by the most recent posting attempt, if present.
    pub(crate) async fn last_token_hash(&self) -> StoreResult<Option<String>> {
        self.with_store("latest_history_token", |store| {
            let page = store.history_page(None, Some(1), 1)?;
            Ok(page
                .records
                .into_iter()
                .last()
                .map(|item| item.record.token_hash))
        })
        .await
    }

    /// Return one bounded page in chronological order.
    pub(crate) async fn history_page(
        &self,
        limit: Option<usize>,
        position: Option<HistoryPosition>,
    ) -> StoreResult<HistoryPage> {
        self.with_store("history", move |store| {
            store.history_page(position, limit, HISTORY_READ_PAGE_SIZE)
        })
        .await
    }

    async fn append_history(&self, record: HistoryRecord) -> StoreResult<()> {
        self.with_store("append_history", move |store| store.append_history(&record))
            .await
    }
}
