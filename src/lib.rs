//! Shared types for the Comenq project.
//!
//! This library defines data structures exchanged between the client
//! and daemon.

use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;

/// Default Unix Domain Socket path for the Comenq daemon.
///
/// Shared by the daemon and CLI to avoid configuration drift.
pub const DEFAULT_SOCKET_PATH: &str = "/run/comenq/comenq.sock";

/// Environment variable naming the per-user runtime directory.
const XDG_RUNTIME_DIR: &str = "XDG_RUNTIME_DIR";

/// Socket location relative to a runtime directory.
const SOCKET_RELATIVE_PATH: &str = "comenq/comenq.sock";

/// Socket path within the per-user runtime directory, when one is available.
///
/// Returns `None` when `XDG_RUNTIME_DIR` is unset or empty, which is the
/// case for system services and non-session processes.
///
/// # Examples
///
/// ```rust,no_run
/// if let Some(path) = comenq_lib::user_socket_path() {
///     println!("user socket would live at {}", path.display());
/// }
/// ```
#[must_use]
pub fn user_socket_path() -> Option<PathBuf> {
    env::var_os(XDG_RUNTIME_DIR)
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(dir).join(SOCKET_RELATIVE_PATH))
}

/// Default socket path for the current execution context.
///
/// Prefers the per-user runtime directory so a user-hosted daemon needs no
/// configuration, falling back to [`DEFAULT_SOCKET_PATH`] when no user
/// runtime directory is available (for example under a system service).
///
/// # Examples
///
/// ```rust,no_run
/// let path = comenq_lib::default_socket_path();
/// println!("daemon will listen on {}", path.display());
/// ```
#[must_use]
pub fn default_socket_path() -> PathBuf {
    user_socket_path().unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET_PATH))
}

/// Candidate sockets a client should try, in preference order.
///
/// Returns the per-user runtime path (when a user runtime directory is
/// available) followed by the system path. Callers must probe candidates by
/// actually connecting, in order, rather than checking for file existence: a
/// daemon that exits without unlinking its socket leaves a stale file
/// behind, and an existence check would select it even though nothing is
/// listening, shadowing a healthy daemon at the next candidate.
///
/// # Examples
///
/// ```rust,no_run
/// for path in comenq_lib::socket_candidates() {
///     println!("would try {}", path.display());
/// }
/// ```
#[must_use]
pub fn socket_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = user_socket_path().into_iter().collect();
    let system = PathBuf::from(DEFAULT_SOCKET_PATH);
    if !candidates.contains(&system) {
        candidates.push(system);
    }
    candidates
}

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
    //! Unit tests for [`CommentRequest`] serialization.
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
            serde_json::to_value(&request).unwrap_or_else(|e| panic!("serialization failed: {e}"));
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

    #[expect(
        clippy::expect_used,
        reason = "tests should fail loudly when fixture setup fails"
    )]
    mod socket_path {

        //! Unit tests for socket path resolution and discovery.
        use crate::{
            DEFAULT_SOCKET_PATH, default_socket_path, socket_candidates, user_socket_path,
        };
        use std::path::PathBuf;
        use test_support::EnvVarGuard;

        #[serial_test::serial]
        #[test]
        fn user_socket_path_requires_runtime_dir() {
            let _guard = EnvVarGuard::remove("XDG_RUNTIME_DIR");
            assert_eq!(user_socket_path(), None);
        }

        #[serial_test::serial]
        #[test]
        fn user_socket_path_ignores_empty_runtime_dir() {
            let _guard = EnvVarGuard::set("XDG_RUNTIME_DIR", "");
            assert_eq!(user_socket_path(), None);
        }

        #[serial_test::serial]
        #[test]
        fn default_socket_path_prefers_runtime_dir() {
            let _guard = EnvVarGuard::set("XDG_RUNTIME_DIR", "/run/user/1000");
            assert_eq!(
                default_socket_path(),
                PathBuf::from("/run/user/1000/comenq/comenq.sock")
            );
        }

        #[serial_test::serial]
        #[test]
        fn default_socket_path_falls_back_to_system_path() {
            let _guard = EnvVarGuard::remove("XDG_RUNTIME_DIR");
            assert_eq!(default_socket_path(), PathBuf::from(DEFAULT_SOCKET_PATH));
        }

        #[serial_test::serial]
        #[test]
        fn socket_candidates_prefer_the_user_socket() {
            let dir = tempfile::tempdir().expect("create tempdir");
            let _guard = EnvVarGuard::set(
                "XDG_RUNTIME_DIR",
                dir.path().to_str().expect("tempdir path is UTF-8"),
            );
            assert_eq!(
                socket_candidates(),
                vec![
                    dir.path().join("comenq/comenq.sock"),
                    PathBuf::from(DEFAULT_SOCKET_PATH),
                ]
            );
        }

        #[serial_test::serial]
        #[test]
        fn socket_candidates_fall_back_to_the_system_path_alone() {
            let _guard = EnvVarGuard::remove("XDG_RUNTIME_DIR");
            assert_eq!(
                socket_candidates(),
                vec![PathBuf::from(DEFAULT_SOCKET_PATH)]
            );
        }

        #[serial_test::serial]
        #[test]
        fn socket_candidates_deduplicate_identical_paths() {
            let _guard = EnvVarGuard::set("XDG_RUNTIME_DIR", "/run/comenq");
            // Contrived: the user path resolves inside /run/comenq, but the
            // system path must not be listed twice if they ever coincide.
            let candidates = socket_candidates();
            let system_count = candidates
                .iter()
                .filter(|p| **p == PathBuf::from(DEFAULT_SOCKET_PATH))
                .count();
            assert!(system_count <= 1);
        }
    }
}
