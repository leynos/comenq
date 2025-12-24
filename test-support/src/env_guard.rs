//! Test helpers for managing environment variables.
//!
//! `EnvVarGuard` temporarily sets an environment variable and restores the
//! previous value on drop.

#[derive(Debug)]
pub struct EnvVarGuard {
    key: String,
    original: Option<String>,
}

impl EnvVarGuard {
    /// Set an environment variable for the lifetime of the returned guard.
    pub fn set(key: &str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        set_env_var(key, value);
        Self {
            key: key.to_string(),
            original,
        }
    }

    /// Remove an environment variable for the lifetime of the returned guard.
    ///
    /// On drop the original value is restored if it was previously set.
    pub fn remove(key: &str) -> Self {
        let original = std::env::var(key).ok();
        remove_env_var(key);
        Self {
            key: key.to_string(),
            original,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(v) => set_env_var(&self.key, v),
            None => remove_env_var(&self.key),
        }
    }
}

/// Set an environment variable for internal guard use.
///
/// SAFETY: Must only be called from serial test contexts.
/// External callers should use `EnvVarGuard::set()` instead.
pub(crate) fn set_env_var(key: &str, value: &str) {
    unsafe { std::env::set_var(key, value) };
}

/// Remove an environment variable for internal guard use.
///
/// SAFETY: Must only be called from serial test contexts.
/// External callers should use `EnvVarGuard::remove()` instead.
pub(crate) fn remove_env_var(key: &str) {
    unsafe { std::env::remove_var(key) };
}

#[cfg(test)]
mod tests {
    #[test]
    #[serial_test::serial]
    fn set_env_var_sets_variable() {
        let key = "ENV_GUARD_SET";
        super::remove_env_var(key);
        assert!(std::env::var(key).is_err());

        super::set_env_var(key, "value");
        assert_eq!(std::env::var(key).unwrap(), "value");

        super::remove_env_var(key);
    }

    #[test]
    #[serial_test::serial]
    fn remove_env_var_removes_variable() {
        let key = "ENV_GUARD_REMOVE";
        super::set_env_var(key, "to_remove");
        assert_eq!(std::env::var(key).unwrap(), "to_remove");

        super::remove_env_var(key);
        assert!(std::env::var(key).is_err());
    }

    #[test]
    #[serial_test::serial]
    fn remove_env_var_when_unset_is_noop() {
        let key = "ENV_GUARD_REMOVE_UNSET";
        super::remove_env_var(key);
        assert!(std::env::var(key).is_err());
    }

    #[test]
    #[serial_test::serial]
    fn env_var_guard_remove_restores_value() {
        let key = "ENV_GUARD_REMOVE_GUARD";
        super::set_env_var(key, "original");
        assert_eq!(std::env::var(key).expect("set"), "original");

        {
            let _guard = super::EnvVarGuard::remove(key);
            assert!(std::env::var(key).is_err());
        }

        assert_eq!(std::env::var(key).expect("restored"), "original");
        super::remove_env_var(key);
    }

    #[test]
    #[serial_test::serial]
    fn env_var_guard_remove_keeps_unset_when_originally_unset() {
        let key = "ENV_GUARD_REMOVE_UNSET_GUARD";
        super::remove_env_var(key);
        assert!(std::env::var(key).is_err());

        {
            let _guard = super::EnvVarGuard::remove(key);
            assert!(std::env::var(key).is_err());
        }

        assert!(std::env::var(key).is_err());
    }

    #[test]
    #[serial_test::serial]
    fn nested_env_var_guard_restores_previous_value() {
        let key = "ENV_GUARD_TEST_NESTED";
        super::remove_env_var(key);

        super::set_env_var(key, "initial");
        assert_eq!(std::env::var(key).unwrap(), "initial");

        let guard1 = super::EnvVarGuard::set(key, "first");
        assert_eq!(std::env::var(key).unwrap(), "first");

        {
            let _guard2 = super::EnvVarGuard::set(key, "second");
            assert_eq!(std::env::var(key).unwrap(), "second");
        }

        assert_eq!(std::env::var(key).unwrap(), "first");

        drop(guard1);
        assert_eq!(std::env::var(key).unwrap(), "initial");

        super::remove_env_var(key);
    }
}
