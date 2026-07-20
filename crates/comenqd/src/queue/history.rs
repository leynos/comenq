//! Posting-history operations on shared queue state.

use crate::store::{HistoryRecord, Result as StoreResult, StoredEntry};

use super::SharedQueue;

impl SharedQueue {
    /// Append a successful posting attempt after its queue entry completes.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use comenqd::daemon::SharedQueue;
    /// # use comenqd::store::StoredEntry;
    /// # async fn record(queue: &SharedQueue, entry: &StoredEntry) -> Result<(), Box<dyn std::error::Error>> {
    /// queue.record_success(entry).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn record_success(&self, entry: &StoredEntry) -> StoreResult<()> {
        self.append_history(HistoryRecord::success(entry, self.clock.unix_now()))
            .await
    }

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
        self.append_history(HistoryRecord::failure(
            entry,
            self.clock.unix_now(),
            error,
        ))
        .await
    }

    /// Return attempts in chronological order, retaining only the newest
    /// `limit` records when a limit is supplied.
    pub(crate) async fn history(
        &self,
        limit: Option<usize>,
    ) -> StoreResult<Vec<HistoryRecord>> {
        self.with_store("history", move |store| {
            let records = store.history()?;
            let skip = limit.map_or(0, |count| records.len().saturating_sub(count));
            Ok(records.into_iter().skip(skip).collect())
        })
        .await
    }

    async fn append_history(&self, record: HistoryRecord) -> StoreResult<()> {
        self.with_store("append_history", move |store| store.append_history(&record))
            .await
    }
}
