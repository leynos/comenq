//! Environment-derived Figment layers for daemon configuration.
//!
//! The loader reads `COMENQD_*` values and the runtime-dependent socket
//! default only through an injected [`Env`], keeping process-wide environment
//! access out of tests.

use mockable::Env;
use ortho_config::figment::{
    providers::Serialized,
    value::{Dict, Value},
};
use std::ffi::OsString;

/// `Config` fields that a `COMENQD_*` environment variable may override.
///
/// Each entry is the serialized field name; [`env_overrides`] uppercases it and
/// prefixes `COMENQD_`, then matches names without regard to case. The list is
/// deliberately explicit so an injected environment provider reads only the
/// documented configuration keys.
pub(super) const ENV_OVERRIDE_FIELDS: [&str; 9] = [
    "github_token",
    "github_token_file",
    "github_token_files",
    "socket_path",
    "queue_path",
    "cooldown_period_seconds",
    "cooldown_flutter_seconds",
    "restart_min_delay_ms",
    "github_api_timeout_secs",
];

/// Lowest-precedence provider for [`super::Config::socket_path`].
///
/// Supplying the environment-aware default as a provider keeps it below the
/// file and environment layers. Serde's static fallback only applies to
/// direct deserialization that bypasses the loader.
pub(super) fn socket_path_defaults(env: &dyn Env) -> Serialized<Dict> {
    let user_socket = comenq_transport::user_socket_path_with_env(env);
    // Only the bounded outcome of the decision is recorded. The path itself
    // is derived from the environment, so logging it would leak raw values.
    tracing::debug!(
        operation = "config_load",
        source = "socket-path-default",
        fallback_reason = if user_socket.is_some() {
            "none"
        } else {
            "runtime-directory-unavailable"
        },
        outcome = "resolved",
        "resolved the default daemon socket path"
    );
    let path = user_socket
        .unwrap_or_else(|| std::path::PathBuf::from(comenq_transport::DEFAULT_SOCKET_PATH));
    let mut values = Dict::new();
    values.insert(
        "socket_path".to_owned(),
        Value::from(path.to_string_lossy().into_owned()),
    );
    Serialized::defaults(values)
}

/// Source of environment variable names and values as native OS strings.
///
/// [`mockable::Env::all`] collects through `std::env::vars`, which panics when
/// any name or value in the process environment is not valid Unicode. The
/// daemon must not fail startup because an unrelated variable carries raw
/// bytes, so [`env_overrides`] enumerates through this narrower port instead,
/// as the `figment::providers::Env` provider it replaces did.
pub(super) trait EnvVars: Send + Sync {
    /// Every variable visible to the process, as OS string pairs.
    fn vars_os(&self) -> Vec<(OsString, OsString)>;
}

/// Production [`EnvVars`] backed by [`std::env::vars_os`].
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct SystemEnvVars;

impl EnvVars for SystemEnvVars {
    fn vars_os(&self) -> Vec<(OsString, OsString)> {
        std::env::vars_os().collect()
    }
}

/// Adapter presenting an injected [`Env`] as an [`EnvVars`] source.
///
/// The injected provider can only represent Unicode values, which is all a
/// test supplies. Production never routes through this adapter.
#[cfg(any(test, feature = "test-support"))]
pub(super) struct EnvVarsFromEnv<'a> {
    env: &'a dyn Env,
}

#[cfg(any(test, feature = "test-support"))]
impl<'a> EnvVarsFromEnv<'a> {
    /// Borrow `env` as an [`EnvVars`] source.
    pub(super) fn new(env: &'a dyn Env) -> Self {
        Self { env }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl EnvVars for EnvVarsFromEnv<'_> {
    fn vars_os(&self) -> Vec<(OsString, OsString)> {
        self.env
            .all()
            .into_iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect()
    }
}

/// Provider for `COMENQD_*` overrides read through `vars`.
///
/// Names match without regard to case, as the previous
/// `figment::providers::Env::prefixed("COMENQD_")` provider did: Figment
/// compares its keys through `UncasedStr`, so an installation that exports
/// `comenqd_socket_path` keeps its override. Names and values are converted
/// with [`OsString::to_string_lossy`], matching that provider's handling of
/// entries the operating system allows but Unicode cannot represent.
///
/// Values use Figment's TOML-like syntax, so numbers and arrays keep their
/// previous meaning. Empty values are passed through because they are
/// legitimate overrides that validation may reject later.
pub(super) fn env_overrides(vars: &dyn EnvVars) -> Serialized<Dict> {
    let available = collect_overrides(vars);
    let values = ENV_OVERRIDE_FIELDS
        .iter()
        .filter_map(|field| {
            let raw = lookup_override(&available, field)?;
            Some(((*field).to_owned(), parse_env_value(raw)))
        })
        .collect::<Dict>();
    // Record how many documented overrides the environment supplied, never
    // the names or values: a value may be a credential.
    tracing::debug!(
        operation = "config_load",
        source = "environment",
        overrides = values.len(),
        outcome = if values.is_empty() { "none" } else { "applied" },
        "resolved COMENQD_* overrides"
    );
    Serialized::defaults(values)
}

/// Convert the environment to trimmed Unicode name/value pairs.
///
/// Trimming mirrors Figment, which trimmed names before matching them.
fn collect_overrides(vars: &dyn EnvVars) -> Vec<(String, String)> {
    vars.vars_os()
        .into_iter()
        .map(|(key, value)| {
            (
                key.to_string_lossy().trim().to_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect()
}

/// Return the value of `COMENQD_<FIELD>` regardless of case.
///
/// An installation that defines several casings gets the documented upper-case
/// name because ASCII ordering sorts it first, so the result is deterministic
/// rather than dependent on enumeration order.
fn lookup_override<'a>(available: &'a [(String, String)], field: &str) -> Option<&'a str> {
    let expected = format!("COMENQD_{}", field.to_ascii_uppercase());
    available
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(&expected))
        .min_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, value)| value.as_str())
}

/// Parse an environment value with Figment's own syntax.
///
/// [`Value`] parsing is infallible; matching [`Infallible`] keeps that
/// guarantee visible without an `unwrap`.
fn parse_env_value(raw: &str) -> Value {
    match raw.parse::<Value>() {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

#[cfg(test)]
mod tests {
    //! Property tests for case-insensitive override lookup.

    use super::{ENV_OVERRIDE_FIELDS, EnvVars, env_overrides, lookup_override, parse_env_value};
    use proptest::prelude::*;
    use std::ffi::OsString;

    /// Environment port over a fixed list of OS-string pairs.
    struct FixedVars(Vec<(OsString, OsString)>);

    impl EnvVars for FixedVars {
        fn vars_os(&self) -> Vec<(OsString, OsString)> {
            self.0.clone()
        }
    }

    /// Apply the requested case flips to the ASCII letters of `name`.
    ///
    /// Non-letters are left alone, so a flip list longer than the name is
    /// harmless. The result always matches `name` case-insensitively.
    fn case_variant(name: &str, flips: &[bool]) -> String {
        name.chars()
            .enumerate()
            .map(|(index, ch)| match flips.get(index).copied() {
                Some(true) if ch.is_ascii_alphabetic() => {
                    if ch.is_ascii_lowercase() {
                        ch.to_ascii_uppercase()
                    } else {
                        ch.to_ascii_lowercase()
                    }
                }
                _ => ch,
            })
            .collect()
    }

    proptest! {
        /// Whatever casing the environment uses, and whatever order the entries
        /// are enumerated in, the documented upper-case name wins.
        ///
        /// This is the invariant installations rely on when several casings of
        /// one variable are exported: the result must not depend on hash or
        /// enumeration order.
        #[test]
        fn upper_case_override_wins_for_every_case_variant_and_order(
            field in prop::sample::select(ENV_OVERRIDE_FIELDS.to_vec()),
            flips in prop::collection::vec(any::<bool>(), 1..32),
            reverse in any::<bool>(),
            variant_value in "[a-z0-9]{1,12}",
            upper_value in "[A-Z0-9]{1,12}",
        ) {
            let upper_name = format!("COMENQD_{}", field.to_ascii_uppercase());
            let variant = case_variant(&upper_name, &flips);
            prop_assume!(variant != upper_name);
            prop_assume!(variant_value != upper_value);

            let mut available = vec![
                (variant, variant_value),
                (upper_name, upper_value.clone()),
            ];
            if reverse {
                available.reverse();
            }

            prop_assert_eq!(
                lookup_override(&available, field),
                Some(upper_value.as_str())
            );

            let vars = FixedVars(
                available
                    .iter()
                    .map(|(name, value)| (OsString::from(name), OsString::from(value)))
                    .collect(),
            );
            let overrides = env_overrides(&vars).value;
            prop_assert_eq!(
                overrides.get(field).cloned(),
                Some(parse_env_value(&upper_value))
            );
        }
    }
}
