//! Behavioural steps for daemon configuration loading.

use cucumber::{World, given, then, when};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use comenqd::config::Config;
use test_support::env_guard::{EnvVarGuard, remove_env_var};

#[derive(Debug, Default, World)]
pub struct ConfigWorld {
    dir: Option<TempDir>,
    path: Option<PathBuf>,
    result: Option<Result<Config, ortho_config::OrthoError>>,
    env_guard: Option<EnvVarGuard>,
}

#[given(regex = r#"^a configuration file with token \"(.+)\"$"#)]
#[expect(clippy::expect_used, reason = "test setup uses expect")]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber requires owned values"
)]
fn config_file_with_token(world: &mut ConfigWorld, token: String) {
    let dir = TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, format!("github_token='{token}'")).expect("write file");
    world.dir = Some(dir);
    world.path = Some(path);
    remove_env_var("COMENQD_SOCKET_PATH");
}

#[given("an invalid configuration file")]
#[expect(clippy::expect_used, reason = "test setup uses expect")]
fn invalid_configuration_file(world: &mut ConfigWorld) {
    let dir = TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc' this is not toml").expect("write file");
    world.dir = Some(dir);
    world.path = Some(path);
}

#[given("a configuration file without github_token")]
#[expect(clippy::expect_used, reason = "test setup uses expect")]
fn config_file_without_token(world: &mut ConfigWorld) {
    let dir = TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "socket_path='/tmp/s.sock'").expect("write file");
    world.dir = Some(dir);
    world.path = Some(path);
}

#[given(regex = r#"^a configuration file with token \"(.+)\" and no socket_path$"#)]
#[expect(clippy::expect_used, reason = "test setup uses expect")]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber requires owned values"
)]
fn config_without_socket(world: &mut ConfigWorld, token: String) {
    let dir = TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        format!("github_token='{token}'\nqueue_path='/tmp/q'"),
    )
    .expect("write file");
    world.dir = Some(dir);
    world.path = Some(path);
    remove_env_var("COMENQD_SOCKET_PATH");
}

#[given(regex = r#"^a configuration file with token \"(.+)\" and no cooldown_period_seconds$"#)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "cucumber requires owned values"
)]
#[expect(
    clippy::redundant_clone,
    reason = "cucumber framework passes tokens by value"
)]
fn config_without_cooldown(world: &mut ConfigWorld, token: String) {
    config_file_with_token(world, token.clone());
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

#[when("the config is loaded")]
#[expect(clippy::expect_used, reason = "test assertions")]
fn load_config(world: &mut ConfigWorld) {
    let path = world.path.as_ref().expect("path set");
    world.result = Some(Config::from_file(path));
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
    }
}
