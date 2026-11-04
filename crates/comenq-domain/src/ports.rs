//! Ports the domain requires from its surroundings.
//!
//! Persistence, time and randomness arrive through these traits, so every
//! adapter decision stays outside the domain.

use crate::model::{ClaimToken, Completion, EntryId, NewEntry, QueuedEntry};
use std::fmt::Debug;

/// Source of wall-clock Unix seconds.
///
/// Implementations must be safe to share across worker and listener tasks.
pub trait WallClock: Debug + Send + Sync {
    /// Seconds since the Unix epoch, truncated to whole seconds.
    fn unix_now(&self) -> u64;
}

/// Source of bounded posting flutter.
///
/// Implementations return a value no greater than `maximum`, inclusive.
pub trait FlutterSampler: Debug + Send + Sync {
    /// Sample a delay in seconds between zero and `maximum`.
    fn sample(&self, maximum: u64) -> u64;
}

/// Durable storage for queue entries and their posting markers.
///
/// The repository owns everything that needs infrastructure: byte budgets,
/// atomic writes, tombstones, completion journalling and replay, claim-token
/// minting and entry lifetimes. The domain should be able to implement every
/// rule using only these operations.
pub trait QueueRepository {
    /// Failure raised by the persistence adapter.
    type Error;

    /// Every valid pending entry, in unspecified order.
    fn entries(&self) -> Result<Vec<QueuedEntry>, Self::Error>;

    /// One entry by identifier, or `None` when no such entry is pending.
    fn find(&self, id: &EntryId) -> Result<Option<QueuedEntry>, Self::Error>;

    /// Durably add `entry`, returning it with its repository-minted lifetime.
    ///
    /// Implementations enforce durable admission limits here.
    fn insert(&self, entry: NewEntry) -> Result<QueuedEntry, Self::Error>;

    /// Replace an existing entry with `entry`.
    fn save(&self, entry: &QueuedEntry) -> Result<(), Self::Error>;

    /// Remove the identified entry and record its awaiting clients' fate.
    fn remove(&self, id: &EntryId) -> Result<(), Self::Error>;

    /// Unix time of the most recent successful post, when any.
    fn last_post(&self) -> Result<Option<u64>, Self::Error>;

    /// Unix time of the most recent failed post attempt, when any.
    fn last_attempt(&self) -> Result<Option<u64>, Self::Error>;

    /// Persist `attempted_at` as the latest failed posting attempt.
    fn record_attempt(&self, attempted_at: u64) -> Result<(), Self::Error>;

    /// Mint an opaque token for one claim.
    fn mint_claim_token(&self) -> ClaimToken;

    /// Record a successful post and retire the claimed entry.
    fn complete(&self, completion: &Completion) -> Result<(), Self::Error>;
}

impl<T: QueueRepository + ?Sized> QueueRepository for &T {
    type Error = T::Error;

    fn entries(&self) -> Result<Vec<QueuedEntry>, Self::Error> {
        (**self).entries()
    }

    fn find(&self, id: &EntryId) -> Result<Option<QueuedEntry>, Self::Error> {
        (**self).find(id)
    }

    fn insert(&self, entry: NewEntry) -> Result<QueuedEntry, Self::Error> {
        (**self).insert(entry)
    }

    fn save(&self, entry: &QueuedEntry) -> Result<(), Self::Error> {
        (**self).save(entry)
    }

    fn remove(&self, id: &EntryId) -> Result<(), Self::Error> {
        (**self).remove(id)
    }

    fn last_post(&self) -> Result<Option<u64>, Self::Error> {
        (**self).last_post()
    }

    fn last_attempt(&self) -> Result<Option<u64>, Self::Error> {
        (**self).last_attempt()
    }

    fn record_attempt(&self, attempted_at: u64) -> Result<(), Self::Error> {
        (**self).record_attempt(attempted_at)
    }

    fn mint_claim_token(&self) -> ClaimToken {
        (**self).mint_claim_token()
    }

    fn complete(&self, completion: &Completion) -> Result<(), Self::Error> {
        (**self).complete(completion)
    }
}

impl<T: WallClock + ?Sized> WallClock for &T {
    fn unix_now(&self) -> u64 {
        (**self).unix_now()
    }
}

impl<T: FlutterSampler + ?Sized> FlutterSampler for &T {
    fn sample(&self, maximum: u64) -> u64 {
        (**self).sample(maximum)
    }
}
