//! Shared types for the Comenq project.
//!
//! This library defines data structures exchanged between the client
//! and daemon.

use serde::{Deserialize, Serialize};

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
        let value = match serde_json::to_value(&request) {
            Ok(v) => v,
            Err(e) => panic!("failed to serialise: {e}"),
        };
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
}
