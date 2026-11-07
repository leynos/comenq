//! Mockable environment providers shared by unit and behavioural tests.

use mockable::MockEnv;
use std::collections::HashMap;
use std::env::VarError;
use std::ffi::OsString;
use std::path::PathBuf;

/// Build an independent [`MockEnv`] from environment key/value pairs.
///
/// Each supported accessor reads from the same map and accepts any number of
/// calls, so a test can focus on the environment values relevant to its case.
///
/// # Examples
///
/// ```rust
/// use mockable::Env as _;
/// use test_support::mock_env;
///
/// let env = mock_env([("XDG_RUNTIME_DIR", "/run/user/1000")]);
/// assert_eq!(env.string("XDG_RUNTIME_DIR").as_deref(), Some("/run/user/1000"));
/// ```
#[must_use]
pub fn mock_env<K, V>(values: impl IntoIterator<Item = (K, V)>) -> MockEnv
where
    K: AsRef<str>,
    V: AsRef<str>,
{
    let values: HashMap<String, String> = values
        .into_iter()
        .map(|(key, value)| (key.as_ref().to_owned(), value.as_ref().to_owned()))
        .collect();
    let mut env = MockEnv::new();

    let string_values = values.clone();
    env.expect_string()
        .returning(move |key| string_values.get(key).cloned())
        .times(..);

    let os_string_values = values.clone();
    env.expect_os_string()
        .returning(move |key| os_string_values.get(key).map(OsString::from))
        .times(..);

    let path_values = values.clone();
    env.expect_path_buf()
        .returning(move |key| path_values.get(key).map(PathBuf::from))
        .times(..);

    let raw_values = values.clone();
    env.expect_raw()
        .returning(move |key| raw_values.get(key).cloned().ok_or(VarError::NotPresent))
        .times(..);

    env.expect_all().returning(move || values.clone()).times(..);

    env
}

#[cfg(test)]
mod tests {
    //! Unit tests for the mock environment helper.

    use super::mock_env;
    use mockable::Env as _;
    use std::{env::VarError, ffi::OsString, path::PathBuf};

    /// Every accessor must read the same map so tests can mix access styles.
    #[test]
    fn mock_env_reads_every_supported_accessor_from_the_same_map() {
        let env = mock_env([("KEY", "value")]);

        assert_eq!(env.string("KEY").as_deref(), Some("value"));
        assert_eq!(env.os_string("KEY"), Some(OsString::from("value")));
        assert_eq!(env.path_buf("KEY"), Some(PathBuf::from("value")));
        assert_eq!(env.raw("KEY"), Ok("value".to_owned()));
        assert_eq!(env.raw("MISSING"), Err(VarError::NotPresent));
        assert_eq!(env.all().get("KEY").map(String::as_str), Some("value"));
    }
}
