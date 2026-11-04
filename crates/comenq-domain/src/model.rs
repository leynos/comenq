//! Queue entries and the values that describe a comment's posting lifetime.

use crate::error::InvalidEntryId;
use std::fmt;
use std::str::FromStr;

/// A comment waiting to be posted to GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Repository owner.
    pub owner: String,
    /// Repository name.
    pub repo: String,
    /// Pull request number.
    pub pr_number: u64,
    /// Full comment body.
    pub body: String,
}

impl Comment {
    /// Build a comment from its posting fields.
    #[must_use]
    pub fn new(
        owner: impl Into<String>,
        repo: impl Into<String>,
        pr_number: u64,
        body: impl Into<String>,
    ) -> Self {
        Self {
            owner: owner.into(),
            repo: repo.into(),
            pr_number,
            body: body.into(),
        }
    }

    /// Check that the repository owner and name are safe to persist and render.
    ///
    /// Returns the offending field label, using the historical wire names
    /// `owner` and `name`.
    ///
    /// # Errors
    ///
    /// Returns `"owner"` or `"name"` when that component is empty, a dot
    /// segment, or contains characters outside ASCII alphanumerics, `.`, `_`
    /// and `-`.
    ///
    /// # Examples
    ///
    /// ```
    /// use comenq_domain::Comment;
    ///
    /// assert!(Comment::new("octocat", "hello-world", 1, "body").validate().is_ok());
    /// assert_eq!(
    ///     Comment::new("..", "hello-world", 1, "body").validate(),
    ///     Err("owner")
    /// );
    /// ```
    pub fn validate(&self) -> Result<(), &'static str> {
        if !is_safe_repository_component(&self.owner) {
            return Err("owner");
        }
        if !is_safe_repository_component(&self.repo) {
            return Err("name");
        }
        Ok(())
    }
}

/// Report whether a GitHub owner or repository name is safe at this boundary.
fn is_safe_repository_component(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// A validated queue-entry identifier: exactly eight ASCII hexadecimal digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryId(String);

impl EntryId {
    /// Validate `value` as a queue-entry identifier.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidEntryId`] unless the value is exactly eight ASCII
    /// hexadecimal characters.
    ///
    /// # Examples
    ///
    /// ```
    /// use comenq_domain::EntryId;
    ///
    /// assert_eq!(
    ///     EntryId::parse("00ffee11").map(|id| id.as_str().to_owned()),
    ///     Ok("00ffee11".into())
    /// );
    /// assert!(EntryId::parse("not-an-id").is_err());
    /// ```
    pub fn parse(value: &str) -> Result<Self, InvalidEntryId> {
        if value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidEntryId::new(value))
        }
    }

    /// Build an identifier from text already proven to be eight hex digits.
    pub(crate) fn from_hex_digits(value: String) -> Self {
        Self(value)
    }

    /// The identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the identifier and return its text.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for EntryId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for EntryId {
    type Err = InvalidEntryId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// An opaque lifetime that distinguishes successive uses of one identifier.
///
/// The repository mints and preserves these; the domain only carries them so
/// completion can refer to the exact queued lifetime.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntryLifetime(String);

impl EntryLifetime {
    /// Wrap a repository-minted lifetime value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The lifetime value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EntryLifetime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// An opaque token identifying one in-flight claim.
///
/// The repository mints the token and persists it; the worker returns it when
/// releasing or completing the claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClaimToken(String);

impl ClaimToken {
    /// Wrap a repository-minted claim token.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The token text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClaimToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A candidate entry handed to a repository for durable insertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEntry {
    /// Deterministic identifier.
    pub id: EntryId,
    /// Explicit queue position; lower posts first.
    pub order: i64,
    /// Flutter sampled when the entry was enqueued, in seconds.
    pub flutter_seconds: u64,
    /// Unix time the entry was enqueued, in seconds.
    pub enqueued_at: u64,
    /// Earliest Unix time the entry may post.
    pub not_before: u64,
    /// The comment to post.
    pub comment: Comment,
}

/// A queue entry as stored and later returned by a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedEntry {
    /// Deterministic identifier.
    pub id: EntryId,
    /// Opaque lifetime that distinguishes reused identifiers.
    pub lifetime: EntryLifetime,
    /// Explicit queue position; lower posts first.
    pub order: i64,
    /// Flutter sampled when the entry was enqueued, in seconds.
    pub flutter_seconds: u64,
    /// Unix time the entry was enqueued, in seconds.
    pub enqueued_at: u64,
    /// Earliest Unix time the entry may post.
    pub not_before: u64,
    /// Claim token while a worker is posting this entry.
    pub claim_token: Option<ClaimToken>,
    /// The comment to post.
    pub comment: Comment,
}

impl QueuedEntry {
    /// Whether a worker currently holds this entry's claim.
    #[must_use]
    pub fn is_claimed(&self) -> bool {
        self.claim_token.is_some()
    }
}

/// A pending entry paired with its projected seconds-until-post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledEntry {
    /// The queued entry.
    pub entry: QueuedEntry,
    /// Whole seconds until the entry is due, never negative.
    pub eta_seconds: u64,
}

/// Everything a repository needs to record one successful post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The claimed entry that was posted.
    pub entry: QueuedEntry,
    /// Hex SHA-256 of the token that made the attempt.
    pub token_hash: String,
    /// Full GitHub comment URL, when the API returned one.
    pub comment_url: Option<String>,
    /// Unix time the post was confirmed.
    pub posted_at: u64,
}
