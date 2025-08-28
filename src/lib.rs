//! Shared types for the Comenq project.
//!
//! This library defines data structures exchanged between the client
//! and daemon.

use serde::{Deserialize, Serialize};

/// Default Unix Domain Socket path for the Comenq daemon.
///
/// Shared by the daemon and CLI to avoid configuration drift.
pub const DEFAULT_SOCKET_PATH: &str = "/run/comenq/comenq.sock";

/// Request sent from the client to the daemon.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommentRequest {
    /// Repository owner.
    pub owner: String,
    /// Repository name.
    pub repo: String,
    /// Pull request number.
    pub pr_number: u64,
    /// Comment body.
    pub body: String,
}

#[cfg(test)]
mod tests {
    use super::CommentRequest;
    use serde_json::{self, json};

    #[test]
    fn serialises_to_json() {
        let request = CommentRequest {
            owner: "octocat".into(),
            repo: "hello-world".into(),
            pr_number: 1,
            body: "Hi".into(),
        };
        let value =
            serde_json::to_value(&request).unwrap_or_else(|e| panic!("serialisation failed: {e}"));
        let expected = json!({
            "owner": "octocat",
            "repo": "hello-world",
            "pr_number": 1,
            "body": "Hi"
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn fails_to_parse_invalid_json() {
        let data = "{ invalid json }";
        let result: Result<CommentRequest, _> = serde_json::from_str(data);
        assert!(result.is_err());
    }

    #[test]
    fn fails_to_parse_missing_fields() {
        let data = r#"{"owner": "octocat"}"#;
        let result: Result<CommentRequest, _> = serde_json::from_str(data);
        assert!(result.is_err());
    }

    #[test]
    fn fails_to_parse_incorrect_field_types() {
        let data = r#"{
            "owner": "octocat",
            "repo": "hello-world",
            "pr_number": "not a number",
            "body": "Hi"
        }"#;
        let result: Result<CommentRequest, _> = serde_json::from_str(data);
        assert!(result.is_err());
    }
}
