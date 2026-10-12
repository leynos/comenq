//! Posting-history operations on shared queue state.

use crate::store::{
    HistoryPage, HistoryPosition, HistoryRecord, Result as StoreResult, StoredEntry,
};

use super::SharedQueue;

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
    /// queue.record_failure(entry, "request timed out").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn record_failure(&self, entry: &StoredEntry, error: &str) -> StoreResult<()> {
        self.append_history(HistoryRecord::failure(entry, self.clock.unix_now(), error))
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
