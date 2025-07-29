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
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(v) => set_env_var(&self.key, v),
            None => remove_env_var(&self.key),
        }
    }
}

/// Safely set an environment variable for tests.
pub fn set_env_var(key: &str, value: &str) {
    // Safety: tests execute serially so no concurrent access occurs.
    unsafe { std::env::set_var(key, value) };
}

/// Safely remove an environment variable for tests.
pub fn remove_env_var(key: &str) {
    // Safety: tests execute serially so no concurrent access occurs.
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
        std::env::set_var(key, "to_remove");
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
    fn nested_env_var_guard_restores_previous_value() {
        let key = "ENV_GUARD_TEST_NESTED";
        super::remove_env_var(key);

        std::env::set_var(key, "initial");
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
