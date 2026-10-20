//! Shared Unix-socket transport policy for the Comenq adapters.
//!
//! The client and daemon use these helpers to resolve compatible per-user and
//! system socket paths through an injectable environment port.

use mockable::{DefaultEnv, Env};
use std::path::PathBuf;

/// Default Unix Domain Socket path for the Comenq daemon.
pub const DEFAULT_SOCKET_PATH: &str = "/run/comenq/comenq.sock";

/// Environment variable naming the per-user runtime directory.
const XDG_RUNTIME_DIR: &str = "XDG_RUNTIME_DIR";
/// Socket location relative to a runtime directory.
const SOCKET_RELATIVE_PATH: &str = "comenq/comenq.sock";

/// Socket path within the per-user runtime directory, when one is available.
///
/// Returns `None` when `XDG_RUNTIME_DIR` is unset, empty, relative, or not
/// valid Unicode in the default environment.
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
    user_socket_path_with_env(&DefaultEnv)
}

/// Socket path within the per-user runtime directory provided by `env`.
///
/// Returns `None` when `XDG_RUNTIME_DIR` is unset, empty, or relative.
///
/// # Examples
///
/// ```rust
/// use mockable::DefaultEnv;
/// use comenq_transport::user_socket_path_with_env;
///
/// if let Some(path) = user_socket_path_with_env(&DefaultEnv) {
///     println!("{}", path.display());
/// }
/// ```
#[must_use]
pub fn user_socket_path_with_env(env: &dyn Env) -> Option<PathBuf> {
    env.os_string(XDG_RUNTIME_DIR)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(SOCKET_RELATIVE_PATH))
}

/// Default socket path for the current execution context.
///
/// Prefers the per-user runtime path, falling back to [`DEFAULT_SOCKET_PATH`].
/// The default environment treats a non-Unicode runtime directory as absent.
///
/// # Examples
///
/// ```rust,no_run
/// let path = comenq_transport::default_socket_path();
/// println!("{}", path.display());
/// ```
#[must_use]
pub fn default_socket_path() -> PathBuf {
    default_socket_path_with_env(&DefaultEnv)
}

/// Default socket path for the environment provided by `env`.
///
/// Prefers the per-user runtime path, falling back to [`DEFAULT_SOCKET_PATH`].
///
/// # Examples
///
/// ```rust
/// use mockable::DefaultEnv;
/// use comenq_transport::default_socket_path_with_env;
///
/// let path = default_socket_path_with_env(&DefaultEnv);
/// println!("{}", path.display());
/// ```
#[must_use]
pub fn default_socket_path_with_env(env: &dyn Env) -> PathBuf {
    user_socket_path_with_env(env).unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET_PATH))
}

/// Candidate sockets a client should try, in preference order.
///
/// Callers connect to candidates rather than checking for socket files, so a
/// stale user socket cannot shadow a live system daemon.
/// The default environment treats a non-Unicode runtime directory as absent.
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
    socket_candidates_with_env(&DefaultEnv)
}

/// Candidate sockets the environment provided by `env` should try.
///
/// The user socket appears first when `XDG_RUNTIME_DIR` is a non-empty
/// absolute path; a duplicate system path is emitted only once.
///
/// # Examples
///
/// ```rust
/// use mockable::DefaultEnv;
/// use comenq_transport::socket_candidates_with_env;
///
/// assert!(!socket_candidates_with_env(&DefaultEnv).is_empty());
/// ```
#[must_use]
pub fn socket_candidates_with_env(env: &dyn Env) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = user_socket_path_with_env(env).into_iter().collect();
    let system = PathBuf::from(DEFAULT_SOCKET_PATH);
    if !candidates.contains(&system) {
        candidates.push(system);
    }
    candidates
}

#[cfg(test)]
mod tests {
    //! Unit tests for shared socket transport policy.

    use super::{
        DEFAULT_SOCKET_PATH, default_socket_path_with_env, socket_candidates_with_env,
        user_socket_path_with_env,
    };
    use rstest::rstest;
    use std::path::PathBuf;
    use test_support::mock_env;

    #[rstest]
    #[case::unset(vec![])]
    #[case::empty(vec![("XDG_RUNTIME_DIR", "")])]
    #[case::relative(vec![("XDG_RUNTIME_DIR", "relative")])]
    fn user_socket_path_requires_an_absolute_runtime_directory(#[case] values: Vec<(&str, &str)>) {
        let env = mock_env(values);
        assert_eq!(user_socket_path_with_env(&env), None);
    }

    #[test]
    fn daemon_default_prefers_the_user_runtime_directory() {
        let env = mock_env([("XDG_RUNTIME_DIR", "/run/user/1000")]);
        assert_eq!(
            default_socket_path_with_env(&env),
            PathBuf::from("/run/user/1000/comenq/comenq.sock")
        );
    }

    #[test]
    fn daemon_default_falls_back_to_the_system_path() {
        let env = mock_env([] as [(&str, &str); 0]);
        assert_eq!(
            default_socket_path_with_env(&env),
            PathBuf::from(DEFAULT_SOCKET_PATH)
        );
    }

    #[test]
    #[expect(
        clippy::expect_used,
        reason = "tests should fail loudly when fixture setup fails"
    )]
    fn candidates_prefer_the_user_socket_and_deduplicate_the_system_path() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let runtime_dir = dir.path().to_str().expect("tempdir path is UTF-8");
        let env = mock_env([("XDG_RUNTIME_DIR", runtime_dir)]);
        assert_eq!(
            socket_candidates_with_env(&env),
            vec![
                dir.path().join("comenq/comenq.sock"),
                PathBuf::from(DEFAULT_SOCKET_PATH),
            ]
        );

        let env = mock_env([("XDG_RUNTIME_DIR", "/run")]);
        assert_eq!(
            socket_candidates_with_env(&env),
            vec![PathBuf::from(DEFAULT_SOCKET_PATH)]
        );
    }
}
