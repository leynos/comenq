//! Request and response types exchanged between the `comenq` client and the
//! `comenqd` daemon over the Unix domain socket.
//!
//! Every connection carries exactly one JSON-encoded [`Request`]; the daemon
//! replies with one JSON-encoded [`Response`] and closes the connection.

use serde::{Deserialize, Serialize};

use crate::CommentRequest;

/// Operation requested by the client.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Enqueue a new comment.
    Put {
        /// The comment to enqueue.
        request: CommentRequest,
    },
    /// List pending comments in posting order.
    List,
    /// Move the identified entry to the head of the queue.
    Bump {
        /// Identifier printed by `list` and `put`.
        id: String,
    },
    /// Move the identified entry to the tail of the queue.
    Bust {
        /// Identifier printed by `list` and `put`.
        id: String,
    },
    /// Remove the identified entry from the queue.
    Del {
        /// Identifier printed by `list` and `put`.
        id: String,
    },
}

/// A pending queue entry as reported by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingEntry {
    /// Deterministic eight-character identifier.
    pub id: String,
    /// Approximate seconds until the comment is posted.
    pub eta_seconds: u64,
    /// Repository owner.
    pub owner: String,
    /// Repository name.
    pub repo: String,
    /// Pull request number.
    pub pr_number: u64,
    /// Full comment body; consumers truncate for display.
    pub body: String,
}

/// Daemon reply to a [`Request`].
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Response {
    /// The request succeeded.
    Ok {
        /// Entry affected by `put`, when applicable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry: Option<PendingEntry>,
        /// Pending entries, returned by `list`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entries: Option<Vec<PendingEntry>>,
    },
    /// The request failed; `message` explains why.
    Error {
        /// Human-readable failure description.
        message: String,
    },
}

impl Response {
    /// Successful reply carrying no payload.
    #[must_use]
    pub fn ok() -> Self {
        Self::Ok {
            entry: None,
            entries: None,
        }
    }

    /// Successful reply for `put`, echoing the enqueued entry.
    #[must_use]
    pub fn entry(entry: PendingEntry) -> Self {
        Self::Ok {
            entry: Some(entry),
            entries: None,
        }
    }

    /// Successful reply for `list`.
    #[must_use]
    pub fn entries(entries: Vec<PendingEntry>) -> Self {
        Self::Ok {
            entry: None,
            entries: Some(entries),
        }
    }

    /// Failed reply with a description.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Serialization round-trip tests for the client-daemon protocol.
    use super::{PendingEntry, Request, Response};
    use crate::CommentRequest;

    fn sample_entry() -> PendingEntry {
        PendingEntry {
            id: "0011aabb".into(),
            eta_seconds: 120,
            owner: "octocat".into(),
            repo: "hello-world".into(),
            pr_number: 7,
            body: "Hi".into(),
        }
    }

    #[test]
    fn put_round_trips_through_json() {
        let req = Request::Put {
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body: "Hi".into(),
            },
        };
        let json = serde_json::to_string(&req).unwrap_or_else(|e| panic!("serialize: {e}"));
        assert!(json.contains(r#""op":"put""#), "missing op tag: {json}");
        let back: Request =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
        assert_eq!(back, req);
    }

    #[test]
    fn id_operations_round_trip_through_json() {
        for req in [
            Request::List,
            Request::Bump {
                id: "0011aabb".into(),
            },
            Request::Bust {
                id: "0011aabb".into(),
            },
            Request::Del {
                id: "0011aabb".into(),
            },
        ] {
            let json = serde_json::to_string(&req).unwrap_or_else(|e| panic!("serialize: {e}"));
            let back: Request =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
            assert_eq!(back, req);
        }
    }

    #[test]
    fn responses_round_trip_through_json() {
        for resp in [
            Response::ok(),
            Response::entry(sample_entry()),
            Response::entries(vec![sample_entry()]),
            Response::error("nope"),
        ] {
            let json = serde_json::to_string(&resp).unwrap_or_else(|e| panic!("serialize: {e}"));
            let back: Response =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
            assert_eq!(back, resp);
        }
    }

    #[test]
    fn unknown_operation_fails_to_parse() {
        let result: Result<Request, _> = serde_json::from_str(r#"{"op":"zap"}"#);
        assert!(result.is_err());
    }
}
