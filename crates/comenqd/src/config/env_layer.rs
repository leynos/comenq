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

/// `Config` fields that a `COMENQD_*` environment variable may override.
///
/// Each entry is the serialized field name; [`env_overrides`] uppercases it and
/// prefixes `COMENQD_`. The list is deliberately explicit so an injected
/// environment provider reads only the documented configuration keys.
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
/// Figment must see a value because removing the serde default made the field
/// mandatory. Supplying it as a provider keeps the environment-dependent
/// default below the file and environment layers.
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
/// Values use Figment's TOML-like syntax, so numbers and arrays keep their
/// previous meaning. Empty values are passed through because they are
/// legitimate overrides that validation may reject later.
pub(super) fn env_overrides(env: &dyn Env) -> Serialized<Dict> {
    let values = ENV_OVERRIDE_FIELDS
        .iter()
        .filter_map(|field| {
            let key = format!("COMENQD_{}", field.to_ascii_uppercase());
            let raw = env.string(&key)?;
            Some(((*field).to_owned(), parse_env_value(&raw)))
        })
        .collect::<Dict>();
    Serialized::defaults(values)
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
