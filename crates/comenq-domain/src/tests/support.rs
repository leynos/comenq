//! In-memory ports and fixtures for domain tests.
//!
//! These doubles live beside the tests rather than in a shared support crate,
//! so the domain crate keeps a single dependency and its tests cannot reach
//! for socket, protocol or filesystem helpers.

use crate::{
    ClaimToken, Comment, Completion, EntryId, EntryLifetime, FlutterSampler, NewEntry, QueuePolicy,
    QueueRepository, QueueService, QueuedEntry, WallClock,
};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Failure injected by [`MemoryRepository`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestError {
    /// Operation that was made to fail.
    pub operation: &'static str,
}

impl TestError {
    /// Warn the test failure for the named operation.
    fn new(operation: &'static str) -> Self {
        Self { operation }
    }
}

impl fmt::Display for TestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "test repository failure: {}", self.operation)
    }
}

impl std::error::Error for TestError {}

/// Mutable state owned by the in-memory repository.
#[derive(Debug, Default)]
pub struct MemoryState {
    /// Entries currently queued.
    pub entries: Vec<QueuedEntry>,
    /// Latest successful post marker.
    pub last_post: Option<u64>,
    /// Latest failed attempt marker.
    pub last_attempt: Option<u64>,
    /// Completions passed by the service.
    pub completions: Vec<Completion>,
    /// Operation name that should fail on its next call.
    pub failure: Option<&'static str>,
    next_lifetime: u64,
    next_claim: u64,
}

/// Repository double with per-operation failure injection.
#[derive(Debug, Default)]
pub struct MemoryRepository {
    state: Mutex<MemoryState>,
}

impl MemoryRepository {
    /// An empty repository.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed an entry directly, bypassing insertion rules.
    pub fn seed(&self, entry: QueuedEntry) {
        self.state().entries.push(entry);
    }

    /// Replace the last successful post marker.
    pub fn set_last_post(&self, value: Option<u64>) {
        self.state().last_post = value;
    }

    /// Replace the last failed attempt marker.
    pub fn set_last_attempt(&self, value: Option<u64>) {
        self.state().last_attempt = value;
    }

    /// Fail the named operation whenever it is called.
    pub fn fail_on(&self, operation: &'static str) {
        self.state().failure = Some(operation);
    }

    /// Snapshot the entries currently stored.
    #[must_use]
    pub fn stored_entries(&self) -> Vec<QueuedEntry> {
        self.state().entries.clone()
    }

    /// Snapshot the completions recorded by the service.
    #[must_use]
    pub fn completions(&self) -> Vec<Completion> {
        self.state().completions.clone()
    }

    fn state(&self) -> MutexGuard<'_, MemoryState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl QueueRepository for MemoryRepository {
    type Error = TestError;

    fn entries(&self) -> Result<Vec<QueuedEntry>, Self::Error> {
        let state = self.state();
        check(&state, "entries")?;
        Ok(state.entries.clone())
    }

    fn find(&self, id: &EntryId) -> Result<Option<QueuedEntry>, Self::Error> {
        let state = self.state();
        check(&state, "find")?;
        Ok(state.entries.iter().find(|entry| entry.id == *id).cloned())
    }

    fn insert(&self, entry: NewEntry) -> Result<QueuedEntry, Self::Error> {
        let mut state = self.state();
        check(&state, "insert")?;
        let queued = QueuedEntry {
            id: entry.id,
            lifetime: EntryLifetime::new(format!("lifetime-{}", state.next_lifetime)),
            order: entry.order,
            flutter_seconds: entry.flutter_seconds,
            enqueued_at: entry.enqueued_at,
            not_before: entry.not_before,
            claim_token: None,
            comment: entry.comment,
        };
        state.next_lifetime = state.next_lifetime.saturating_add(1);
        state.entries.push(queued.clone());
        Ok(queued)
    }

    fn save(&self, entry: &QueuedEntry) -> Result<(), Self::Error> {
        let mut state = self.state();
        check(&state, "save")?;
        state.entries.retain(|stored| stored.id != entry.id);
        state.entries.push(entry.clone());
        Ok(())
    }

    fn remove(&self, id: &EntryId) -> Result<(), Self::Error> {
        let mut state = self.state();
        check(&state, "remove")?;
        state.entries.retain(|entry| entry.id != *id);
        Ok(())
    }

    fn last_post(&self) -> Result<Option<u64>, Self::Error> {
        let state = self.state();
        check(&state, "last_post")?;
        Ok(state.last_post)
    }

    fn last_attempt(&self) -> Result<Option<u64>, Self::Error> {
        let state = self.state();
        check(&state, "last_attempt")?;
        Ok(state.last_attempt)
    }

    fn record_attempt(&self, attempted_at: u64) -> Result<(), Self::Error> {
        let mut state = self.state();
        check(&state, "record_attempt")?;
        state.last_attempt = Some(state.last_attempt.unwrap_or(0).max(attempted_at));
        Ok(())
    }

    fn mint_claim_token(&self) -> ClaimToken {
        let mut state = self.state();
        state.next_claim = state.next_claim.saturating_add(1);
        ClaimToken::new(format!("token-{}", state.next_claim))
    }

    fn complete(&self, completion: &Completion) -> Result<(), Self::Error> {
        let mut state = self.state();
        check(&state, "complete")?;
        state.last_post = Some(state.last_post.unwrap_or(0).max(completion.posted_at));
        state
            .entries
            .retain(|entry| entry.id != completion.entry.id);
        state.completions.push(completion.clone());
        Ok(())
    }
}

fn check(state: &MemoryState, operation: &'static str) -> Result<(), TestError> {
    if state.failure == Some(operation) {
        Err(TestError::new(operation))
    } else {
        Ok(())
    }
}

/// Clock fixed at one instant, counting how often it is read.
#[derive(Debug, Clone)]
pub struct FixedClock {
    now: u64,
    reads: Arc<AtomicU64>,
}

impl FixedClock {
    /// Fix the clock at `now`.
    #[must_use]
    pub fn new(now: u64) -> Self {
        Self {
            now,
            reads: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Number of times the clock has been read.
    #[must_use]
    pub fn reads(&self) -> u64 {
        self.reads.load(Ordering::SeqCst)
    }
}

impl WallClock for FixedClock {
    fn unix_now(&self) -> u64 {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.now
    }
}

/// Sampler returning one fixed delay and remembering the requested maximum.
#[derive(Debug, Clone)]
pub struct FixedFlutter {
    sample: u64,
    last_maximum: Arc<Mutex<Option<u64>>>,
}

impl FixedFlutter {
    /// Return `sample` for every request.
    #[must_use]
    pub fn new(sample: u64) -> Self {
        Self {
            sample,
            last_maximum: Arc::new(Mutex::new(None)),
        }
    }

    /// The maximum passed to the most recent sample, when sampled.
    #[must_use]
    pub fn last_maximum(&self) -> Option<u64> {
        *self
            .last_maximum
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl FlutterSampler for FixedFlutter {
    fn sample(&self, maximum: u64) -> u64 {
        *self
            .last_maximum
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(maximum);
        self.sample.min(maximum)
    }
}

/// Repository, clock, sampler and policy used by most tests.
pub struct Fixture {
    /// In-memory repository.
    pub repository: MemoryRepository,
    /// Fixed clock.
    pub clock: FixedClock,
    /// Fixed sampler.
    pub sampler: FixedFlutter,
    /// Posting policy.
    pub policy: QueuePolicy,
}

impl Fixture {
    /// A fixture with a 600-second cooldown and 240-second flutter bound.
    #[must_use]
    pub fn new(now: u64, flutter: u64) -> Self {
        Self::with_policy(now, flutter, QueuePolicy::new(600, 240))
    }

    /// A fixture with an explicit policy.
    #[must_use]
    pub fn with_policy(now: u64, flutter: u64, policy: QueuePolicy) -> Self {
        Self {
            repository: MemoryRepository::new(),
            clock: FixedClock::new(now),
            sampler: FixedFlutter::new(flutter),
            policy,
        }
    }

    /// A service borrowing this fixture's ports.
    pub fn service(&self) -> QueueService<&MemoryRepository, &FixedClock, &FixedFlutter> {
        QueueService::new(&self.repository, &self.clock, &self.sampler, self.policy)
    }
}

/// Unwrap a successful domain result, failing the test otherwise.
pub fn unwrap_ok<T>(result: Result<T, crate::QueueError<TestError>>) -> T {
    result.unwrap_or_else(|error| panic!("domain operation failed: {error}"))
}

/// Unwrap a failed domain result, failing the test otherwise.
#[must_use]
pub fn unwrap_err<T>(
    result: Result<T, crate::QueueError<TestError>>,
) -> crate::QueueError<TestError> {
    result
        .err()
        .unwrap_or_else(|| panic!("domain operation unexpectedly succeeded"))
}

/// Parse a test identifier, failing loudly when the fixture is wrong.
#[must_use]
pub fn entry_id(value: &str) -> EntryId {
    EntryId::parse(value).unwrap_or_else(|error| panic!("test identifier must be valid: {error}"))
}

/// An entry with no claim and a stable lifetime.
#[must_use]
pub fn seeded_entry(
    id: &str,
    order: i64,
    enqueued_at: u64,
    not_before: u64,
    flutter_seconds: u64,
) -> QueuedEntry {
    QueuedEntry {
        id: entry_id(id),
        lifetime: EntryLifetime::new(format!("lifetime-{id}")),
        order,
        flutter_seconds,
        enqueued_at,
        not_before,
        claim_token: None,
        comment: Comment::new("octocat", "hello-world", 7, id),
    }
}

/// A representative comment.
#[must_use]
pub fn comment() -> Comment {
    Comment::new("octocat", "hello-world", 7, "comment")
}
