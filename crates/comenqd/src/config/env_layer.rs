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
use std::collections::HashMap;

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
    let path = comenq_transport::default_socket_path_with_env(env);
    let mut values = Dict::new();
    values.insert(
        "socket_path".to_owned(),
        Value::from(path.to_string_lossy().into_owned()),
    );
    Serialized::defaults(values)
}

/// Provider for `COMENQD_*` overrides read through `env`.
///
/// Names match without regard to case, as the previous
/// `figment::providers::Env::prefixed("COMENQD_")` provider did: Figment
/// compares its keys through `UncasedStr`, so an installation that exports
/// `comenqd_socket_path` keeps its override.
///
/// Values use Figment's TOML-like syntax, so numbers and arrays keep their
/// previous meaning. Empty values are passed through because they are
/// legitimate overrides that validation may reject later.
pub(super) fn env_overrides(env: &dyn Env) -> Serialized<Dict> {
    let available = env.all();
    let values = ENV_OVERRIDE_FIELDS
        .iter()
        .filter_map(|field| {
            let raw = lookup_override(&available, field)?;
            Some(((*field).to_owned(), parse_env_value(raw)))
        })
        .collect::<Dict>();
    Serialized::defaults(values)
}

/// Return the value of `COMENQD_<FIELD>` regardless of case.
///
/// An installation that defines several casings gets the documented upper-case
/// name because ASCII ordering sorts it first, so the result is deterministic
/// rather than dependent on hash iteration order.
fn lookup_override<'a>(available: &'a HashMap<String, String>, field: &str) -> Option<&'a str> {
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
