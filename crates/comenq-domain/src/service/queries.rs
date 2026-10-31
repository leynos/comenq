//! Read-only queue projections: the ordered schedule and the next due entry.
//!
//! Both read the clock once and share one projection over the repository's
//! entries and posting markers.

use crate::error::QueueError;
use crate::model::ScheduledEntry;
use crate::policy::{activity_anchor, project_schedule};
use crate::ports::{FlutterSampler, QueueRepository, WallClock};

use super::QueueService;

impl<R, C, F> QueueService<R, C, F>
where
    R: QueueRepository,
    C: WallClock,
    F: FlutterSampler,
{
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

    /// Whether a projected entry set contains a claim.
    fn has_claim_in(&self, schedule: &[ScheduledEntry]) -> bool {
        schedule
            .iter()
            .any(|scheduled| scheduled.entry.is_claimed())
    }
}
