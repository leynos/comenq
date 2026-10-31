//! Compatibility wrappers that run queue policy through the domain service.
//!
//! The store no longer decides ordering, floors, claims or ETAs. These
//! synchronous methods exist so the persistence layer and its tests can drive
//! the domain service with an explicit clock instant and flutter sample while
//! holding the caller's queue lock.

use comenq_domain::{FlutterSampler, PutComment, QueueError, QueuePolicy, QueueService, WallClock};
use comenq_lib::CommentRequest;
use uuid::Uuid;

use super::{PutOptions, QueueStore, Result, StoreError, StoredEntry};
use crate::queue::convert::{comment_from_request, stored_schedule};

/// Clock fixed at the instant supplied by a synchronous store operation.
#[derive(Debug, Clone, Copy)]
struct FixedClock(u64);

impl WallClock for FixedClock {
    fn unix_now(&self) -> u64 {
        self.0
    }
}

/// Sampler returning the caller's already-sampled flutter delay.
#[derive(Debug, Clone, Copy)]
struct FixedFlutter(u64);

impl FlutterSampler for FixedFlutter {
    fn sample(&self, maximum: u64) -> u64 {
        self.0.min(maximum)
    }
}

/// Compose a domain service over the store with fixed time and flutter.
fn domain_service(
    store: &QueueStore,
    now: u64,
    cooldown: u64,
    flutter_seconds: u64,
) -> QueueService<&QueueStore, FixedClock, FixedFlutter> {
    QueueService::new(
        store,
        FixedClock(now),
        FixedFlutter(flutter_seconds),
        QueuePolicy::new(cooldown, flutter_seconds),
    )
}

/// Collapse a domain error back into the store's historical error enum.
pub(crate) fn store_error_from_queue(error: QueueError<StoreError>) -> StoreError {
    match error {
        QueueError::UnknownIdentifier(id) => StoreError::UnknownId(id),
        QueueError::InvalidIdentifier(error) => StoreError::InvalidId(error.value().to_owned()),
        QueueError::InvalidRepositoryComponent(field) => {
            StoreError::InvalidRepositoryComponent(field)
        }
        QueueError::Repository(error) => error,
    }
}

impl QueueStore {
    /// Enqueue `request` and return the persisted entry with its stable ETA.
    ///
    /// Every fallible scheduling read happens inside the domain service before
    /// the entry is written, so a projection error never leaves an unannounced
    /// queue entry.
    pub fn put_with_eta(
        &self,
        request: CommentRequest,
        options: &PutOptions,
        now: u64,
    ) -> Result<(StoredEntry, u64)> {
        let scheduled = domain_service(self, now, options.cooldown, options.flutter_seconds)
            .put(PutComment {
                comment: comment_from_request(&request),
                immediate: options.immediate,
            })
            .map_err(store_error_from_queue)?;
        Ok(stored_schedule(scheduled))
    }

    /// Enqueue `request` at the tail using its supplied flutter sample.
    pub fn put(
        &self,
        request: CommentRequest,
        options: &PutOptions,
        now: u64,
    ) -> Result<StoredEntry> {
        self.put_with_eta(request, options, now)
            .map(|(entry, _)| entry)
    }

    /// Pending entries paired with their estimated seconds-until-post.
    pub fn schedule(&self, cooldown: u64, now: u64) -> Result<Vec<(StoredEntry, u64)>> {
        Ok(domain_service(self, now, cooldown, 0)
            .schedule()
            .map_err(store_error_from_queue)?
            .into_iter()
            .map(stored_schedule)
            .collect())
    }

    /// The head entry and its estimated seconds-until-post, when any.
    pub fn next_due(&self, cooldown: u64, now: u64) -> Result<Option<(StoredEntry, u64)>> {
        Ok(domain_service(self, now, cooldown, 0)
            .next_due()
            .map_err(store_error_from_queue)?
            .map(stored_schedule))
    }

    /// Claim a due entry while holding the store lock for the whole transition.
    pub fn claim_next_due(&self, cooldown: u64, now: u64) -> Result<Option<(StoredEntry, u64)>> {
        Ok(domain_service(self, now, cooldown, 0)
            .claim_next_due()
            .map_err(store_error_from_queue)?
            .map(stored_schedule))
    }

    /// Release a failed post's claim so the worker can retry it later.
    ///
    /// A missing entry or stale token is accepted as a no-op.
    pub fn release_claim(&self, id: &str, claim_token: &str, attempted_at: u64) -> Result<()> {
        domain_service(self, attempted_at, 0, 0)
            .release_claim(id, claim_token)
            .map(|_| ())
            .map_err(store_error_from_queue)
    }

    /// Move the identified entry to the head of the queue.
    pub fn bump(&self, id: &str) -> Result<()> {
        domain_service(self, 0, 0, 0)
            .bump(id)
            .map_err(store_error_from_queue)
    }

    /// Move the identified entry to the tail of the queue.
    pub fn bust(&self, id: &str) -> Result<()> {
        domain_service(self, 0, 0, 0)
            .bust(id)
            .map_err(store_error_from_queue)
    }

    /// Remove the identified entry from the queue.
    pub fn del(&self, id: &str) -> Result<()> {
        domain_service(self, 0, 0, 0)
            .del(id)
            .map_err(store_error_from_queue)
    }

    /// Clear interrupted claims and assign stable lifetimes to legacy entries.
    pub(super) fn reclaim_claims(&self) -> Result<()> {
        for mut entry in self.entries()? {
            let was_claimed = entry.claim_token.take().is_some();
            let needs_incarnation = entry.incarnation.is_empty();
            if needs_incarnation {
                entry.incarnation = Uuid::new_v4().to_string();
            }
            if was_claimed || needs_incarnation {
                self.write_entry(&entry)?;
            }
        }
        Ok(())
    }

    /// Reconcile an interrupted completion before reclaiming claims from an
    /// earlier worker that used this still-open store.
    pub(crate) fn recover_worker_state(&self) -> Result<()> {
        self.reconcile_completion()?;
        self.reclaim_claims()
    }
}
