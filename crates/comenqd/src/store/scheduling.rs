//! Queue scheduling and atomic enqueue projection.

use super::{PutOptions, QueueStore, Result, StoredEntry};
use comenq_lib::CommentRequest;
use rand::Rng;

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
        let mut entries = self.entries()?;
        let last_post = self.last_post()?;
        if let Some(entry) = existing {
            let eta = projected_schedule(entries, last_post, options.cooldown, now)
                .into_iter()
                .find(|(scheduled, _)| scheduled.id == entry.id)
                .map_or(0, |(_, eta)| eta);
            return Ok((entry, eta));
        }

        let flutter_seconds = if options.flutter_max == 0 {
            0
        } else {
            rand::rng().random_range(0..=options.flutter_max)
        };
        let entry = StoredEntry {
            id,
            order: entries
                .last()
                .map_or(0, |tail| tail.order.saturating_add(1)),
            flutter_seconds,
            enqueued_at: now,
            not_before: if options.immediate {
                0
            } else {
                now.saturating_add(options.cooldown)
                    .saturating_add(flutter_seconds)
            },
            request,
        };
        entries.push(entry.clone());
        let eta = projected_schedule(entries, last_post, options.cooldown, now)
            .into_iter()
            .find(|(scheduled, _)| scheduled.id == entry.id)
            .map_or(0, |(_, eta)| eta);
        self.write_entry(&entry)?;
        Ok((entry, eta))
    }

    /// Enqueue `request` at the tail, sampling its flutter now.
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
        Ok(self.schedule(cooldown, now)?.into_iter().next())
    }
}

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
