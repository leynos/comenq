//! Storage- and transport-independent comment queue domain.
//!
//! This crate owns the queue's business rules: deterministic entry
//! identifiers, enqueue scheduling, reordering, claiming, releasing and
//! completion hand-off. It deliberately knows nothing about JSON, filesystem
//! paths, Tokio tasks, Unix sockets, metrics or daemon configuration.
//!
//! Persistence is supplied through [`QueueRepository`], time through
//! [`WallClock`] and randomness through [`FlutterSampler`]. A
//! [`QueueService`] composes those ports with a [`QueuePolicy`], so the same
//! rules run against an in-memory repository in tests and the daemon's
//! filesystem store in production.
//!
//! # Examples
//!
//! ```rust
//! use comenq_domain::{Comment, EntryId, QueuePolicy};
//!
//! let comment = Comment::new("octocat", "hello-world", 7, "Looks good");
//! assert!(comment.validate().is_ok());
//!
//! let id = EntryId::parse("1a2b3c4d").expect("valid identifier");
//! assert_eq!(id.as_str(), "1a2b3c4d");
//!
//! let policy = QueuePolicy::new(600, 240);
//! assert_eq!(policy.cooldown_seconds, 600);
//! ```

mod error;
mod model;
mod policy;
mod ports;
mod service;

pub use error::{InvalidEntryId, QueueError};
pub use model::{
    ClaimToken, Comment, Completion, EntryId, EntryLifetime, NewEntry, QueuedEntry, ScheduledEntry,
};
pub use policy::{QueuePolicy, activity_anchor, derive_entry_id, project_schedule};
pub use ports::{FlutterSampler, QueueRepository, WallClock};
pub use service::{
    CommandOutcome, PutComment, QueryOutcome, QueueCommand, QueueQuery, QueueService,
    ReleaseOutcome,
};

#[cfg(test)]
mod tests;
