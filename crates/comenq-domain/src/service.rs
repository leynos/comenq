//! Queue commands and queries composed from ports and the posting policy.
//!
//! Operation implementations live in [`commands`] and [`queries`]; this module
//! holds the value types, the service value, and the dispatch entry points.

use crate::error::QueueError;
use crate::model::{ClaimToken, Comment, EntryId, QueuedEntry, ScheduledEntry};
use crate::policy::QueuePolicy;
use crate::ports::{FlutterSampler, QueueRepository, WallClock};

mod commands;
mod queries;

/// A validated enqueue request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutComment {
    /// The comment to enqueue.
    pub comment: Comment,
    /// Post as soon as the queue allows, bypassing only the enqueue floor.
    ///
    /// Claim exclusion and the cooldown between posts still apply.
    pub immediate: bool,
}

/// Result of releasing a failed post's claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// A matching claim was cleared and the attempt time recorded.
    Released,
    /// The entry or token no longer matched, so nothing changed.
    Stale,
}

/// A state-changing queue request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueCommand {
    /// Claim the due head entry when nothing else is claimed.
    ClaimNextDue,
    /// Enqueue a comment.
    Put(PutComment),
    /// Move the identified entry to the head of the queue.
    Bump(EntryId),
    /// Move the identified entry to the tail of the queue.
    Bust(EntryId),
    /// Remove the identified entry.
    Del(EntryId),
    /// Clear a failed post's claim after a matching token.
    ReleaseClaim {
        /// Entry that was claimed.
        id: EntryId,
        /// Token minted for the claim.
        token: ClaimToken,
    },
    /// Record a successful post; the service stamps the posting time.
    Complete {
        /// The claimed entry that was posted.
        entry: QueuedEntry,
        /// Hex SHA-256 of the token that made the attempt.
        token_hash: String,
        /// Full GitHub comment URL, when the API returned one.
        comment_url: Option<String>,
    },
}

/// Value produced by a successful [`QueueCommand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    /// The head entry; claimed only when its wait is zero.
    Claimed(Option<ScheduledEntry>),
    /// The enqueued entry and its projected wait.
    Put(ScheduledEntry),
    /// A reorder or removal completed.
    Changed,
    /// The claim-release result.
    ReleaseClaim(ReleaseOutcome),
    /// The completion was recorded.
    Completed,
}

/// A read-only queue request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueQuery {
    /// Every pending entry with its projected wait.
    List,
    /// The head entry, when nothing is claimed.
    NextDue,
}

/// Value produced by a successful [`QueueQuery`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryOutcome {
    /// Pending entries in posting order.
    List(Vec<ScheduledEntry>),
    /// The head entry, when one may be posted.
    NextDue(Option<ScheduledEntry>),
}

/// Queue rules applied to a repository, clock and flutter sampler.
///
/// The service holds no locks or I/O itself. Callers that need atomic
/// read-modify-write sequences must hold an external lock around the whole
/// service call.
#[derive(Debug)]
pub struct QueueService<R, C, F> {
    repository: R,
    clock: C,
    sampler: F,
    policy: QueuePolicy,
}

impl<R, C, F> QueueService<R, C, F>
where
    R: QueueRepository,
    C: WallClock,
    F: FlutterSampler,
{
    /// Compose a service from its ports and policy.
    #[must_use]
    pub fn new(repository: R, clock: C, sampler: F, policy: QueuePolicy) -> Self {
        Self {
            repository,
            clock,
            sampler,
            policy,
        }
    }

    /// The repository behind this service.
    #[must_use]
    pub fn repository(&self) -> &R {
        &self.repository
    }

    /// Execute one command.
    ///
    /// # Errors
    ///
    /// Returns the error of the corresponding command method.
    pub fn execute(&self, command: QueueCommand) -> Result<CommandOutcome, QueueError<R::Error>> {
        match command {
            QueueCommand::ClaimNextDue => self.claim_next_due().map(CommandOutcome::Claimed),
            QueueCommand::Put(request) => self.put(request).map(CommandOutcome::Put),
            QueueCommand::Bump(id) => self.bump(id.as_str()).map(|()| CommandOutcome::Changed),
            QueueCommand::Bust(id) => self.bust(id.as_str()).map(|()| CommandOutcome::Changed),
            QueueCommand::Del(id) => self.del(id.as_str()).map(|()| CommandOutcome::Changed),
            QueueCommand::ReleaseClaim { id, token } => self
                .release_claim(id.as_str(), token.as_str())
                .map(CommandOutcome::ReleaseClaim),
            QueueCommand::Complete {
                entry,
                token_hash,
                comment_url,
            } => self
                .complete(entry, token_hash, comment_url)
                .map(|()| CommandOutcome::Completed),
        }
    }

    /// Answer one query.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::Repository`] when the repository cannot supply
    /// the values needed to answer it.
    pub fn query(&self, query: QueueQuery) -> Result<QueryOutcome, QueueError<R::Error>> {
        match query {
            QueueQuery::List => self.schedule().map(QueryOutcome::List),
            QueueQuery::NextDue => self.next_due().map(QueryOutcome::NextDue),
        }
    }
}
