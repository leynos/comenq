//! Conversions from the shared test-support configuration type.
//!
//! Test harnesses construct [`super::Config`] from
//! [`test_support::daemon::TestConfig`]. Keeping those impls beside the type
//! but in their own module stops the loader module from growing past its size
//! limit as the loader gains seams.

use super::{Config, DEFAULT_COOLDOWN_FLUTTER};

/// Convert a [`test_support::daemon::TestConfig`] into a [`Config`].
///
/// This implementation is only available when the `test-support` feature is
/// enabled and allows `TestConfig` to be converted via `Config::from(test_cfg)`
/// or `test_cfg.into()`.
///
/// ```rust,no_run
/// use comenqd::config::Config;
/// use test_support::temp_config;
///
/// let tmp = tempfile::tempdir().expect("create tempdir");
/// let cfg: Config = temp_config(&tmp).into();
/// assert_eq!(cfg.cooldown_period_seconds, 1);
/// ```
#[cfg_attr(docsrs, doc(cfg(feature = "test-support")))]
impl From<test_support::daemon::TestConfig> for Config {
    fn from(value: test_support::daemon::TestConfig) -> Self {
        let test_support::daemon::TestConfig {
            github_token,
            socket_path,
            queue_path,
            cooldown_period_seconds,
            restart_min_delay_ms,
            github_api_timeout_secs,
        } = value;
        Self {
            github_token,
            github_token_file: None,
            github_token_files: Vec::new(),
            socket_path,
            queue_path,
            cooldown_period_seconds,
            cooldown_flutter_seconds: DEFAULT_COOLDOWN_FLUTTER,
            restart_min_delay_ms,
            github_api_timeout_secs,
        }
    }
}

/// Convert a borrowed [`test_support::daemon::TestConfig`] into a [`Config`].
///
/// Requires the `test-support` feature and clones only the owned fields,
/// avoiding an intermediate allocation of a full
/// [`test_support::daemon::TestConfig`] clone.
///
/// # Examples
///
/// ```rust,no_run
/// use comenqd::config::Config;
/// use tempfile::tempdir;
/// use test_support::temp_config;
///
/// let tmp = tempdir().expect("create temp directory");
/// let test_cfg = temp_config(&tmp);
///
/// // Convert by reference without cloning.
/// let cfg = Config::from(&test_cfg);
///
/// assert_eq!(cfg.socket_path, test_cfg.socket_path);
/// assert_eq!(cfg.queue_path, test_cfg.queue_path);
/// ```
#[cfg_attr(docsrs, doc(cfg(feature = "test-support")))]
impl From<&test_support::daemon::TestConfig> for Config {
    fn from(value: &test_support::daemon::TestConfig) -> Self {
        Self {
            github_token: value.github_token.clone(),
            github_token_file: None,
            github_token_files: Vec::new(),
            socket_path: value.socket_path.clone(),
            queue_path: value.queue_path.clone(),
            cooldown_period_seconds: value.cooldown_period_seconds,
            cooldown_flutter_seconds: DEFAULT_COOLDOWN_FLUTTER,
            restart_min_delay_ms: value.restart_min_delay_ms,
            github_api_timeout_secs: value.github_api_timeout_secs,
        }
    }
}
