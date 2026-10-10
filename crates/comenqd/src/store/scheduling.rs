//! Queue scheduling and atomic enqueue projection.

use super::{PutOptions, QueueStore, Result, StoreError, StoredEntry};
use crate::store::MAX_QUEUE_ENTRIES;
use comenq_lib::CommentRequest;
use uuid::Uuid;

impl QueueStore {
    /// Enqueue `request` and return the persisted entry with its stable ETA.
    ///
    /// Every fallible scheduling read happens before the entry is written, so
    /// a response-projection error never leaves an unannounced queue entry.
    pub fn put_with_eta(
        &self,
        request: CommentRequest,
        options: &PutOptions,
        now: u64,
    ) -> Result<(StoredEntry, u64)> {
        let (id, existing) = self.resolve_entry_id(&request, now)?;
        let (mut entries, usage) = self.entries_with_usage()?;
        let last_post = self.last_post()?;
        if let Some(entry) = existing {
            let eta = projected_schedule(entries, last_post, options.cooldown, now)
                .into_iter()
                .find(|(scheduled, _)| scheduled.id == entry.id)
                .map_or(0, |(_, eta)| eta);
            return Ok((entry, eta));
        }
        if entries.len() >= MAX_QUEUE_ENTRIES {
            return Err(StoreError::QueueFull(MAX_QUEUE_ENTRIES));
        }

        let entry = StoredEntry {
            id,
            order: entries
                .last()
                .map_or(0, |tail| tail.order.saturating_add(1)),
            flutter_seconds: options.flutter_seconds,
            enqueued_at: now,
            not_before: if options.immediate {
                0
            } else {
                now.saturating_add(options.cooldown)
                    .saturating_add(options.flutter_seconds)
            },
            claim_token: None,
            request,
        };
        entries.push(entry.clone());
        let eta = projected_schedule(entries, last_post, options.cooldown, now)
            .into_iter()
            .find(|(scheduled, _)| scheduled.id == entry.id)
            .map_or(0, |(_, eta)| eta);
        self.write_entry_with_usage(&entry, usage)?;
        Ok((entry, eta))
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
        Ok(projected_schedule(
            self.entries()?,
            self.last_post()?,
            cooldown,
            now,
        ))
    }

    /// The head entry and its estimated seconds-until-post, when any.
    pub fn next_due(&self, cooldown: u64, now: u64) -> Result<Option<(StoredEntry, u64)>> {
        let schedule = self.schedule(cooldown, now)?;
        if schedule
            .iter()
            .any(|(entry, _)| entry.claim_token.is_some())
        {
            return Ok(None);
        }
        Ok(schedule.into_iter().next())
    }

    /// Claim a due entry while holding the store lock for the whole transition.
    pub fn claim_next_due(&self, cooldown: u64, now: u64) -> Result<Option<(StoredEntry, u64)>> {
        let Some((mut entry, wait_seconds)) = self.next_due(cooldown, now)? else {
            return Ok(None);
        };
        if wait_seconds > 0 {
            return Ok(Some((entry, wait_seconds)));
        }
        entry.claim_token = Some(Uuid::new_v4().to_string());
        self.write_entry(&entry)?;
        Ok(Some((entry, 0)))
    }

    /// Release a failed post's claim so the worker can retry it later.
    pub fn release_claim(&self, id: &str, claim_token: &str) -> Result<()> {
        let mut entry = match self.find(id) {
            Ok(entry) => entry,
            Err(StoreError::UnknownId(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
        if entry.claim_token.as_deref() == Some(claim_token) {
            entry.claim_token = None;
            self.write_entry(&entry)?;
        }
        Ok(())
    }

    /// Clear claims left by a worker that exited before completing its post.
    pub(super) fn reclaim_claims(&self) -> Result<()> {
        for mut entry in self.entries()? {
            if entry.claim_token.take().is_some() {
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

/// Project ordered entries onto their earliest posting times.
///
/// The input entries must already be in posting order; each projected time is
/// constrained by the previous post, the entry's stored floor, and `now`.
fn projected_schedule(
    entries: Vec<StoredEntry>,
    mut previous_post: Option<u64>,
    cooldown: u64,
    now: u64,
) -> Vec<(StoredEntry, u64)> {
    entries
        .into_iter()
        .map(|entry| {
            let due = previous_post.map_or(now, |previous| {
                previous
                    .saturating_add(cooldown)
                    .saturating_add(entry.flutter_seconds)
            });
            let post_at = due.max(entry.not_before).max(now);
            previous_post = Some(post_at);
            (entry, post_at.saturating_sub(now))
        })
        .collect()
}
