//! Behavioural steps for daemon configuration loading.

use anyhow::Context as _;
use cucumber::{World, given, then, when};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use comenqd::config::Config;
use test_support::EnvVarGuard;

#[derive(Debug, Default, World)]
pub struct ConfigWorld {
    dir: Option<TempDir>,
    path: Option<PathBuf>,
    result: Option<Result<Config, ortho_config::OrthoError>>,
    env_guard: Option<EnvVarGuard>,
    socket_guard: Option<EnvVarGuard>,
}

/// Create a tempdir containing a `config.toml` with `content`.
fn write_temp_config(content: &str) -> anyhow::Result<(TempDir, PathBuf)> {
    let dir = TempDir::new().context("create temp dir")?;
    let path = dir.path().join("config.toml");
    fs::write(&path, content).context("write file")?;
    Ok((dir, path))
}

#[given(regex = r#"^a configuration file with token \"(.+)\"$"#)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber requires owned values"
)]
fn config_file_with_token(world: &mut ConfigWorld, token: String) -> anyhow::Result<()> {
    let (dir, path) = write_temp_config(&format!("github_token='{token}'"))?;
    world.dir = Some(dir);
    world.path = Some(path);
    world.socket_guard = Some(EnvVarGuard::remove("COMENQD_SOCKET_PATH"));
    Ok(())
}

#[given("an invalid configuration file")]
fn invalid_configuration_file(world: &mut ConfigWorld) -> anyhow::Result<()> {
    let (dir, path) = write_temp_config("github_token='abc' this is not toml")?;
    world.dir = Some(dir);
    world.path = Some(path);
    Ok(())
}

#[given("a configuration file without github_token")]
fn config_file_without_token(world: &mut ConfigWorld) -> anyhow::Result<()> {
    let (dir, path) = write_temp_config("socket_path='/tmp/s.sock'")?;
    world.dir = Some(dir);
    world.path = Some(path);
    Ok(())
}

#[given(regex = r#"^a configuration file with token \"(.+)\" and no socket_path$"#)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber requires owned values"
)]
fn config_without_socket(world: &mut ConfigWorld, token: String) -> anyhow::Result<()> {
    let (dir, path) = write_temp_config(&format!("github_token='{token}'\nqueue_path='/tmp/q'"))?;
    world.dir = Some(dir);
    world.path = Some(path);
    world.socket_guard = Some(EnvVarGuard::remove("COMENQD_SOCKET_PATH"));
    Ok(())
}

#[given(regex = r#"^a configuration file with token \"(.+)\" and no cooldown_period_seconds$"#)]
fn config_without_cooldown(world: &mut ConfigWorld, token: String) -> anyhow::Result<()> {
    config_file_with_token(world, token)
}

#[given(regex = r#"^a configuration file referencing a token file containing \"(.+)\"$"#)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber requires owned values"
)]
fn config_with_token_file(world: &mut ConfigWorld, token: String) -> anyhow::Result<()> {
    let dir = TempDir::new().context("create temp dir")?;
    let token_path = dir.path().join("token");
    fs::write(&token_path, format!("{token}\n")).context("write token file")?;
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!("github_token_file='{}'", token_path.display()),
    )
    .context("write config")?;
    world.dir = Some(dir);
    world.path = Some(path);
    Ok(())
}

#[given("a configuration file referencing a missing token file")]
fn config_with_missing_token_file(world: &mut ConfigWorld) -> anyhow::Result<()> {
    let (dir, path) = write_temp_config("github_token_file='/nonexistent/token'")?;
    world.dir = Some(dir);
    world.path = Some(path);
    Ok(())
}

#[given("a missing configuration file")]
fn missing_configuration_file(world: &mut ConfigWorld) {
    world.path = Some(PathBuf::from("/nonexistent/nowhere.toml"));
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber requires owned values"
)]
#[given(regex = r#"^environment variable \"(.+)\" is \"(.+)\"$"#)]
fn set_env_var(world: &mut ConfigWorld, key: String, value: String) {
    world.env_guard = Some(EnvVarGuard::set(&key, &value));
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber requires owned values"
)]
#[given(regex = r#"^environment variable \"(.+)\" is unset$"#)]
fn unset_env_var(world: &mut ConfigWorld, key: String) {
    world.env_guard = Some(EnvVarGuard::remove(&key));
}

#[when("the config is loaded")]
fn load_config(world: &mut ConfigWorld) -> anyhow::Result<()> {
    let path = world.path.as_ref().context("path set")?;
    world.result = Some(Config::from_file(path));
    Ok(())
}

#[then(regex = r#"^github token is \"(.+)\"$"#)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber requires owned values"
)]
fn github_token_is(world: &mut ConfigWorld, expected: String) {
    match world.result.take() {
        Some(Ok(cfg)) => assert_eq!(cfg.github_token, expected),
        other => panic!("expected success, got {other:?}"),
    }
}

#[then("config loading fails")]
fn config_loading_fails(world: &mut ConfigWorld) {
    match world.result.take() {
        Some(Err(_)) => {}
        other => panic!("expected error, got {other:?}"),
    }
}

#[then(regex = r#"^socket path is \"(.+)\"$"#)]
fn socket_path_is(world: &mut ConfigWorld, expected: String) {
    match world.result.take() {
        Some(Ok(cfg)) => assert_eq!(cfg.socket_path, PathBuf::from(expected)),
        other => panic!("expected success, got {other:?}"),
    }
}

#[then(regex = r"^cooldown_period_seconds is ([0-9]+)$")]
fn cooldown_period_seconds_is(world: &mut ConfigWorld, expected: u64) {
    match world.result.take() {
        Some(Ok(cfg)) => assert_eq!(cfg.cooldown_period_seconds, expected),
        other => panic!("expected success, got {other:?}"),
    }
}

impl Drop for ConfigWorld {
    fn drop(&mut self) {
        if let Some(_guard) = self.env_guard.take() {
            // dropping the guard restores the previous state
        }
        if let Some(_guard) = self.socket_guard.take() {
            // dropping the guard restores the previous state
        }
    }
}
