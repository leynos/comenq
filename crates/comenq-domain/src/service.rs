//! Queue commands and queries composed from ports and the posting policy.

use crate::error::QueueError;
use crate::model::{
    ClaimToken, Comment, Completion, EntryId, EntryLifetime, NewEntry, QueuedEntry, ScheduledEntry,
};
use crate::policy::{QueuePolicy, activity_anchor, derive_entry_id_with_salt, project_schedule};
use crate::ports::{FlutterSampler, QueueRepository, WallClock};

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
    /// Record a successful post.
    Complete(Completion),
}

/// Value produced by a successful [`QueueCommand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
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

    /// Enqueue a comment, or return the existing entry for a repeated put.
    ///
    /// Every fallible read completes before the new entry is persisted, so a
    /// failed projection never leaves an unannounced queue entry. Sampling the
    /// flutter delay happens once, before those reads.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::InvalidRepositoryComponent`] for an unsafe
    /// owner or repository name, and [`QueueError::Repository`] when the
    /// repository cannot complete a read or the insert.
    pub fn put(&self, request: PutComment) -> Result<ScheduledEntry, QueueError<R::Error>> {
        request
            .comment
            .validate()
            .map_err(QueueError::InvalidRepositoryComponent)?;
        let now = self.clock.unix_now();
        let flutter_seconds = self.sampler.sample(self.policy.flutter_max_seconds);
        let entries = self.repository.entries()?;
        let previous_activity = activity_anchor(
            self.repository.last_post()?,
            self.repository.last_attempt()?,
        );
        let mut salt = None;
        let (id, existing) = loop {
            let id = derive_entry_id_with_salt(&request.comment, now, salt);
            match self.repository.find(&id)? {
                Some(existing) if existing.comment == request.comment => {
                    break (id, Some(existing));
                }
                Some(_) => salt = Some(salt.map_or(1, |value| value.saturating_add(1))),
                None => break (id, None),
            }
        };
        if let Some(existing) = existing {
            return Ok(self.scheduled_entry(entries, previous_activity, now, &existing));
        }

        let mut ordered = entries;
        ordered.sort_by(entry_order);
        let order = ordered
            .last()
            .map_or(0, |tail| tail.order.saturating_add(1));
        let provisional = QueuedEntry {
            id: id.clone(),
            lifetime: EntryLifetime::default(),
            order,
            flutter_seconds,
            enqueued_at: now,
            not_before: if request.immediate {
                0
            } else {
                now.saturating_add(self.policy.cooldown_seconds)
                    .saturating_add(flutter_seconds)
            },
            claim_token: None,
            comment: request.comment.clone(),
        };
        ordered.push(provisional.clone());
        let eta_seconds = project_schedule(
            ordered,
            previous_activity,
            self.policy.cooldown_seconds,
            now,
        )
        .into_iter()
        .find(|scheduled| scheduled.entry.id == id)
        .map_or(0, |scheduled| scheduled.eta_seconds);

        let entry = self.repository.insert(NewEntry {
            id,
            order,
            flutter_seconds,
            enqueued_at: now,
            not_before: provisional.not_before,
            comment: request.comment,
        })?;
        Ok(ScheduledEntry { entry, eta_seconds })
    }

    /// Every pending entry paired with its projected wait.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::Repository`] when the repository cannot supply
    /// its entries or activity markers.
    pub fn schedule(&self) -> Result<Vec<ScheduledEntry>, QueueError<R::Error>> {
        let now = self.clock.unix_now();
        self.project(now)
    }

    /// The head entry and its projected wait, when nothing is claimed.
    ///
    /// A claim anywhere in the queue suppresses dequeueing, so the head is
    /// returned unclaimed while another worker posts.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::Repository`] when the repository cannot supply
    /// its entries or activity markers.
    pub fn next_due(&self) -> Result<Option<ScheduledEntry>, QueueError<R::Error>> {
        let schedule = self.project(self.clock.unix_now())?;
        if self.has_claim_in(&schedule) {
            return Ok(None);
        }
        Ok(schedule.into_iter().next())
    }

    /// Claim the due head entry while the caller holds the queue lock.
    ///
    /// The head is returned unclaimed when its wait is still positive, and no
    /// entry is returned while any claim exists.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::Repository`] when the repository cannot read the
    /// queue or persist the claim.
    pub fn claim_next_due(&self) -> Result<Option<ScheduledEntry>, QueueError<R::Error>> {
        let schedule = self.project(self.clock.unix_now())?;
        if self.has_claim_in(&schedule) {
            return Ok(None);
        }
        let Some(mut scheduled) = schedule.into_iter().next() else {
            return Ok(None);
        };
        if scheduled.eta_seconds > 0 {
            return Ok(Some(scheduled));
        }
        scheduled.entry.claim_token = Some(self.repository.mint_claim_token());
        self.repository.save(&scheduled.entry)?;
        Ok(Some(scheduled))
    }

    /// Move the identified entry to the head of the queue.
    ///
    /// Reordering preserves the entry's not-before floor and any claim.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::InvalidIdentifier`] for a malformed identifier,
    /// [`QueueError::UnknownIdentifier`] when no such entry exists, and
    /// [`QueueError::Repository`] on a storage failure.
    pub fn bump(&self, id: &str) -> Result<(), QueueError<R::Error>> {
        let id = parse_id(id)?;
        let mut entries = self.repository.entries()?;
        entries.sort_by(entry_order);
        let mut entry = self.find_required(&id)?;
        entry.order = entries.first().map_or(entry.order, |head| {
            if head.id == entry.id {
                entry.order
            } else {
                head.order.saturating_sub(1)
            }
        });
        self.repository.save(&entry)?;
        Ok(())
    }

    /// Move the identified entry to the tail of the queue.
    ///
    /// Reordering preserves the entry's not-before floor and any claim.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::InvalidIdentifier`] for a malformed identifier,
    /// [`QueueError::UnknownIdentifier`] when no such entry exists, and
    /// [`QueueError::Repository`] on a storage failure.
    pub fn bust(&self, id: &str) -> Result<(), QueueError<R::Error>> {
        let id = parse_id(id)?;
        let mut entries = self.repository.entries()?;
        entries.sort_by(entry_order);
        let mut entry = self.find_required(&id)?;
        entry.order = entries.last().map_or(entry.order, |tail| {
            if tail.id == entry.id {
                entry.order
            } else {
                tail.order.saturating_add(1)
            }
        });
        self.repository.save(&entry)?;
        Ok(())
    }

    /// Remove the identified entry from the queue.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::InvalidIdentifier`] for a malformed identifier,
    /// [`QueueError::UnknownIdentifier`] when no such entry exists, and
    /// [`QueueError::Repository`] on a storage failure.
    pub fn del(&self, id: &str) -> Result<(), QueueError<R::Error>> {
        let id = parse_id(id)?;
        self.find_required(&id)?;
        self.repository.remove(&id)?;
        Ok(())
    }

    /// Release a failed post's claim after a matching token.
    ///
    /// A missing entry or a token that no longer matches is a no-op. A
    /// matching release persists the later of the previous attempt marker and
    /// this attempt before clearing the claim, so the retry respects the
    /// cooldown anchor.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::InvalidIdentifier`] for a malformed identifier
    /// and [`QueueError::Repository`] when the repository cannot persist the
    /// attempt marker or clear the claim.
    pub fn release_claim(
        &self,
        id: &str,
        claim_token: &str,
    ) -> Result<ReleaseOutcome, QueueError<R::Error>> {
        let id = parse_id(id)?;
        let now = self.clock.unix_now();
        let Some(mut entry) = self.repository.find(&id)? else {
            return Ok(ReleaseOutcome::Stale);
        };
        if entry.claim_token.as_ref().map(ClaimToken::as_str) != Some(claim_token) {
            return Ok(ReleaseOutcome::Stale);
        }
        let attempted_at = self.repository.last_attempt()?.unwrap_or(0).max(now);
        self.repository.record_attempt(attempted_at)?;
        entry.claim_token = None;
        self.repository.save(&entry)?;
        Ok(ReleaseOutcome::Released)
    }

    /// Record a successful post for a claimed entry.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::Repository`] when the completion cannot be
    /// durably journalled.
    pub fn complete(
        &self,
        entry: QueuedEntry,
        token_hash: String,
        comment_url: Option<String>,
    ) -> Result<(), QueueError<R::Error>> {
        let completion = Completion {
            entry,
            token_hash,
            comment_url,
            posted_at: self.clock.unix_now(),
        };
        self.repository.complete(&completion)?;
        Ok(())
    }

    /// Execute one command.
    ///
    /// # Errors
    ///
    /// Returns the error of the corresponding command method.
    pub fn execute(&self, command: QueueCommand) -> Result<CommandOutcome, QueueError<R::Error>> {
        match command {
            QueueCommand::Put(request) => self.put(request).map(CommandOutcome::Put),
            QueueCommand::Bump(id) => self.bump(id.as_str()).map(|()| CommandOutcome::Changed),
            QueueCommand::Bust(id) => self.bust(id.as_str()).map(|()| CommandOutcome::Changed),
            QueueCommand::Del(id) => self.del(id.as_str()).map(|()| CommandOutcome::Changed),
            QueueCommand::ReleaseClaim { id, token } => self
                .release_claim(id.as_str(), token.as_str())
                .map(CommandOutcome::ReleaseClaim),
            QueueCommand::Complete(completion) => self
                .complete(
                    completion.entry,
                    completion.token_hash,
                    completion.comment_url,
                )
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

    /// Read the clock once, then project the whole queue.
    fn project(&self, now: u64) -> Result<Vec<ScheduledEntry>, QueueError<R::Error>> {
        let entries = self.repository.entries()?;
        let previous_activity = activity_anchor(
            self.repository.last_post()?,
            self.repository.last_attempt()?,
        );
        Ok(project_schedule(
            entries,
            previous_activity,
            self.policy.cooldown_seconds,
            now,
        ))
    }

    /// Project a known entry set and select one entry's wait.
    fn scheduled_entry(
        &self,
        entries: Vec<QueuedEntry>,
        previous_activity: Option<u64>,
        now: u64,
        entry: &QueuedEntry,
    ) -> ScheduledEntry {
        let eta_seconds = project_schedule(
            entries,
            previous_activity,
            self.policy.cooldown_seconds,
            now,
        )
        .into_iter()
        .find(|scheduled| scheduled.entry.id == entry.id)
        .map_or(0, |scheduled| scheduled.eta_seconds);
        ScheduledEntry {
            entry: entry.clone(),
            eta_seconds,
        }
    }

    /// Whether a projected entry set contains a claim.
    fn has_claim_in(&self, schedule: &[ScheduledEntry]) -> bool {
        schedule
            .iter()
            .any(|scheduled| scheduled.entry.is_claimed())
    }

    /// Find an entry or report the identifier as unknown.
    fn find_required(&self, id: &EntryId) -> Result<QueuedEntry, QueueError<R::Error>> {
        self.repository
            .find(id)?
            .ok_or_else(|| QueueError::UnknownIdentifier(id.as_str().to_owned()))
    }
}

/// Parse a caller-supplied identifier inside the domain.
fn parse_id<E>(id: &str) -> Result<EntryId, QueueError<E>> {
    EntryId::parse(id).map_err(QueueError::InvalidIdentifier)
}

/// Posting order: explicit position, then enqueue time, then identifier.
fn entry_order(left: &QueuedEntry, right: &QueuedEntry) -> std::cmp::Ordering {
    (left.order, left.enqueued_at, left.id.as_str()).cmp(&(
        right.order,
        right.enqueued_at,
        right.id.as_str(),
    ))
}
