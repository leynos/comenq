//! Deterministic identifiers and the queue's posting schedule.
//!
//! The rules here are pure: they take values already read from a repository
//! and return projections without touching storage or reading a clock.

use crate::model::{Comment, EntryId, QueuedEntry, ScheduledEntry};

/// Queue-wide posting policy derived from daemon configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuePolicy {
    /// Minimum spacing between successful posts, in seconds.
    pub cooldown_seconds: u64,
    /// Inclusive upper bound for sampled flutter, in seconds.
    pub flutter_max_seconds: u64,
}

impl QueuePolicy {
    /// Build a policy from the daemon's cooldown and flutter settings.
    ///
    /// # Examples
    ///
    /// ```
    /// use comenq_domain::QueuePolicy;
    ///
    /// let policy = QueuePolicy::new(600, 240);
    /// assert_eq!(policy.cooldown_seconds, 600);
    /// assert_eq!(policy.flutter_max_seconds, 240);
    /// ```
    #[must_use]
    pub fn new(cooldown_seconds: u64, flutter_max_seconds: u64) -> Self {
        Self {
            cooldown_seconds,
            flutter_max_seconds,
        }
    }
}

/// The cooldown anchor: the later of the last successful post and the last
/// failed attempt.
///
/// Scheduling from the attempt time as well as the post time keeps a failed
/// post from being retried sooner than a successful one would have been
/// followed.
///
/// # Examples
///
/// ```
/// use comenq_domain::activity_anchor;
///
/// assert_eq!(activity_anchor(Some(100), Some(130)), Some(130));
/// assert_eq!(activity_anchor(Some(100), None), Some(100));
/// assert_eq!(activity_anchor(None, None), None);
/// ```
#[must_use]
pub fn activity_anchor(last_post: Option<u64>, last_attempt: Option<u64>) -> Option<u64> {
    last_post.into_iter().chain(last_attempt).max()
}

/// Derive a comment's deterministic identifier at one enqueue time.
///
/// The identifier is the first eight lowercase hexadecimal digits of a
/// 64-bit FNV-1a hash over the comment fields and enqueue time. Hashing the
/// values directly, rather than a serialized form, keeps the identifier stable
/// without importing a wire format into the domain.
///
/// # Examples
///
/// ```
/// use comenq_domain::{Comment, derive_entry_id};
///
/// let comment = Comment::new("octocat", "hello-world", 7, "comment");
/// assert_eq!(derive_entry_id(&comment, 1_000).as_str(), "088fc020");
/// ```
#[must_use]
pub fn derive_entry_id(comment: &Comment, enqueued_at: u64) -> EntryId {
    derive_entry_id_with_salt(comment, enqueued_at, None)
}

/// Derive an identifier, adding a salt only when resolving a collision.
pub(crate) fn derive_entry_id_with_salt(
    comment: &Comment,
    enqueued_at: u64,
    salt: Option<u64>,
) -> EntryId {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    let mut eat = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    };
    eat(comment.owner.as_bytes());
    eat(&[0]);
    eat(comment.repo.as_bytes());
    eat(&[0]);
    eat(&comment.pr_number.to_le_bytes());
    eat(comment.body.as_bytes());
    eat(&enqueued_at.to_le_bytes());
    if let Some(salt) = salt {
        eat(&salt.to_le_bytes());
    }
    let hex: String = format!("{hash:016x}").chars().take(8).collect();
    EntryId::from_hex_digits(hex)
}

/// Posting order: explicit position, then enqueue time, then identifier.
pub(crate) fn entry_order(left: &QueuedEntry, right: &QueuedEntry) -> std::cmp::Ordering {
    (left.order, left.enqueued_at, left.id.as_str()).cmp(&(
        right.order,
        right.enqueued_at,
        right.id.as_str(),
    ))
}

/// Project ordered entries onto their earliest posting times.
///
/// Entries are ordered by explicit position, then enqueue time, then
/// identifier. Each projected time is constrained by the previous post, the
/// entry's stored floor, and `now`; arithmetic saturates rather than wrapping.
///
/// # Examples
///
/// ```
/// use comenq_domain::{activity_anchor, project_schedule};
///
/// let schedule = project_schedule(Vec::new(), activity_anchor(Some(100), None), 600, 1_000);
/// assert!(schedule.is_empty());
/// ```
#[must_use]
pub fn project_schedule(
    mut entries: Vec<QueuedEntry>,
    previous_post: Option<u64>,
    cooldown: u64,
    now: u64,
) -> Vec<ScheduledEntry> {
    entries.sort_by(entry_order);
    let mut previous_post = previous_post;
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
            ScheduledEntry {
                entry,
                eta_seconds: post_at.saturating_sub(now),
            }
        })
        .collect()
}
