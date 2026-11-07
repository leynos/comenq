//! OS-native environment access for socket overrides.
//!
//! [`mockable::Env`] exposes environment values only as UTF-8 strings, so a
//! non-Unicode value reads as unset. A Unix socket path may contain arbitrary
//! bytes, so `COMENQ_SOCKET` is read through the narrower [`OsEnv`] port while
//! everything else keeps using the injected [`Env`](mockable::Env).

use mockable::Env;
use std::ffi::OsString;

/// Reads environment variables as native OS strings.
///
/// Implementors decide how a value that is not valid UTF-8 is treated. The
/// production implementation preserves those bytes; a provider that can only
/// represent Unicode reports the variable as absent.
///
/// `Send + Sync` mirrors [`mockable::Env`] so a shared reference can cross an
/// await point in the client's `Send` future.
pub(crate) trait OsEnv: Send + Sync {
    /// Value of `key`, preserving non-Unicode bytes when the provider can.
    fn os_string(&self, key: &str) -> Option<OsString>;
}

/// Production [`OsEnv`] backed by [`std::env::var_os`].
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DefaultOsEnv;

impl OsEnv for DefaultOsEnv {
    fn os_string(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }
}

/// Adapter presenting an injected [`Env`] as an [`OsEnv`].
///
/// Tests use this so an in-memory provider, rather than the process
/// environment, supplies the value.
pub(crate) struct EnvOsString<'a> {
    env: &'a dyn Env,
}

impl<'a> EnvOsString<'a> {
    /// Borrow `env` as an [`OsEnv`] source.
    pub(crate) fn new(env: &'a dyn Env) -> Self {
        Self { env }
    }
}

impl OsEnv for EnvOsString<'_> {
    fn os_string(&self, key: &str) -> Option<OsString> {
        self.env.os_string(key)
    }
}
