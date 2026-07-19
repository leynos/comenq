//! Shared types for the Comenq project.
//!
//! This library defines data structures exchanged between the client
//! and daemon.

use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;

pub mod protocol;

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

/// Discover the socket a client should connect to.
///
/// Returns the first existing socket among the per-user runtime path and the
/// system path, so a client reaches a user-hosted daemon when one is running
/// and otherwise falls back to a system-hosted daemon. When neither socket
/// exists the context-appropriate default is returned so connection errors
/// mention the most likely intended path.
///
/// # Examples
///
/// ```rust,no_run
/// let path = comenq_lib::discover_socket_path();
/// println!("connecting to {}", path.display());
/// ```
#[must_use]
pub fn discover_socket_path() -> PathBuf {
    [user_socket_path(), Some(PathBuf::from(DEFAULT_SOCKET_PATH))]
        .into_iter()
        .flatten()
        .find(|candidate| candidate.exists())
        .unwrap_or_else(default_socket_path)
}

/// Request sent from the client to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
            DEFAULT_SOCKET_PATH, default_socket_path, discover_socket_path, user_socket_path,
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
        fn discover_socket_path_prefers_existing_user_socket() {
            let dir = tempfile::tempdir().expect("create tempdir");
            let socket_dir = dir.path().join("comenq");
            std::fs::create_dir_all(&socket_dir).expect("create socket dir");
            let socket = socket_dir.join("comenq.sock");
            std::fs::write(&socket, b"").expect("create placeholder socket");
            let _guard = EnvVarGuard::set(
                "XDG_RUNTIME_DIR",
                dir.path().to_str().expect("tempdir path is UTF-8"),
            );
            assert_eq!(discover_socket_path(), socket);
        }

        #[serial_test::serial]
        #[test]
        fn discover_socket_path_defaults_when_nothing_exists() {
            let dir = tempfile::tempdir().expect("create tempdir");
            let _guard = EnvVarGuard::set(
                "XDG_RUNTIME_DIR",
                dir.path().to_str().expect("tempdir path is UTF-8"),
            );
            assert_eq!(
                discover_socket_path(),
                dir.path().join("comenq/comenq.sock")
            );
        }
    }
}
