//! Shared Unix-socket transport policy for the Comenq adapters.
//!
//! The client and daemon use these helpers to resolve compatible per-user and
//! system socket paths without coupling the protocol crate to environment or
//! filesystem concerns.

use std::env;
use std::path::PathBuf;

/// Default Unix Domain Socket path for the Comenq daemon.
pub const DEFAULT_SOCKET_PATH: &str = "/run/comenq/comenq.sock";

/// Environment variable naming the per-user runtime directory.
const XDG_RUNTIME_DIR: &str = "XDG_RUNTIME_DIR";
/// Socket location relative to a runtime directory.
const SOCKET_RELATIVE_PATH: &str = "comenq/comenq.sock";

/// Socket path within the per-user runtime directory, when one is available.
///
/// Returns `None` when `XDG_RUNTIME_DIR` is unset, empty, or relative.
///
/// # Examples
///
/// ```rust,no_run
/// if let Some(path) = comenq_transport::user_socket_path() {
///     println!("{}", path.display());
/// }
/// ```
#[must_use]
pub fn user_socket_path() -> Option<PathBuf> {
    env::var_os(XDG_RUNTIME_DIR)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(SOCKET_RELATIVE_PATH))
}

/// Default socket path for the current execution context.
///
/// Prefers the per-user runtime path, falling back to [`DEFAULT_SOCKET_PATH`].
///
/// # Examples
///
/// ```rust,no_run
/// let path = comenq_transport::default_socket_path();
/// println!("{}", path.display());
/// ```
#[must_use]
pub fn default_socket_path() -> PathBuf {
    user_socket_path().unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET_PATH))
}

/// Candidate sockets a client should try, in preference order.
///
/// Callers connect to candidates rather than checking for socket files, so a
/// stale user socket cannot shadow a live system daemon.
///
/// # Examples
///
/// ```rust,no_run
/// for path in comenq_transport::socket_candidates() {
///     println!("{}", path.display());
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

#[cfg(test)]
mod tests {
    //! Unit tests for shared socket transport policy.

    use super::{DEFAULT_SOCKET_PATH, default_socket_path, socket_candidates, user_socket_path};
    use std::path::PathBuf;
    use test_support::EnvVarGuard;

    #[serial_test::serial]
    #[test]
    fn user_socket_path_requires_an_absolute_runtime_directory() {
        let _guard = EnvVarGuard::remove("XDG_RUNTIME_DIR");
        assert_eq!(user_socket_path(), None);

        let _guard = EnvVarGuard::set("XDG_RUNTIME_DIR", "");
        assert_eq!(user_socket_path(), None);

        let _guard = EnvVarGuard::set("XDG_RUNTIME_DIR", "relative");
        assert_eq!(user_socket_path(), None);
    }

    #[serial_test::serial]
    #[test]
    fn daemon_default_prefers_the_user_runtime_directory() {
        let _guard = EnvVarGuard::set("XDG_RUNTIME_DIR", "/run/user/1000");
        assert_eq!(
            default_socket_path(),
            PathBuf::from("/run/user/1000/comenq/comenq.sock")
        );
    }

    #[serial_test::serial]
    #[test]
    fn daemon_default_falls_back_to_the_system_path() {
        let _guard = EnvVarGuard::remove("XDG_RUNTIME_DIR");
        assert_eq!(default_socket_path(), PathBuf::from(DEFAULT_SOCKET_PATH));
    }

    #[serial_test::serial]
    #[test]
    #[expect(
        clippy::expect_used,
        reason = "tests should fail loudly when fixture setup fails"
    )]
    fn candidates_prefer_the_user_socket_and_deduplicate_the_system_path() {
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

        let _guard = EnvVarGuard::set("XDG_RUNTIME_DIR", "/run");
        assert_eq!(
            socket_candidates(),
            vec![PathBuf::from(DEFAULT_SOCKET_PATH)]
        );
    }
}
