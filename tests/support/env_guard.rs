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
