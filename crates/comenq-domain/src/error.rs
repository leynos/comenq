//! Domain errors that map directly to client-facing queue failures.

use std::fmt;

/// An identifier that is not exactly eight ASCII hexadecimal characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidEntryId {
    value: String,
}

impl InvalidEntryId {
    /// Wrap the rejected caller-supplied value.
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    /// The rejected value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for InvalidEntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "queue entry id '{}' must be eight hexadecimal characters",
            self.value
        )
    }
}

impl std::error::Error for InvalidEntryId {}

/// A queue operation failure with storage errors kept separate.
///
/// The messages of the semantic variants match the daemon's historical wire
/// responses exactly, so moving a rule into the domain does not change what a
/// client sees.
#[derive(Debug, thiserror::Error)]
pub enum QueueError<E> {
    /// No queued comment carries the requested identifier.
    #[error("no queued comment has id '{0}'")]
    UnknownIdentifier(String),
    /// The identifier cannot name a queue entry.
    #[error(transparent)]
    InvalidIdentifier(InvalidEntryId),
    /// A GitHub repository owner or name is unsafe to render or persist.
    #[error("GitHub repository {0} contains unsafe characters")]
    InvalidRepositoryComponent(&'static str),
    /// The persistence adapter rejected or failed the operation.
    #[error(transparent)]
    Repository(#[from] E),
}

impl<E> QueueError<E> {
    /// Whether this error originated outside the queue's business rules.
    #[must_use]
    pub fn repository(&self) -> Option<&E> {
        match self {
            Self::Repository(error) => Some(error),
            Self::UnknownIdentifier(_)
            | Self::InvalidIdentifier(_)
            | Self::InvalidRepositoryComponent(_) => None,
        }
    }
}
