//! Behavioural steps for daemon configuration loading.
#![allow(
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::uninlined_format_args,
    reason = "simplify test failure output"
)]

use cucumber::{World, given, then, when};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use comenqd::config::Config;

#[derive(Debug, Default, World)]
pub struct ConfigWorld {
    dir: Option<TempDir>,
    path: Option<PathBuf>,
    result: Option<Result<Config, ortho_config::OrthoError>>,
    env_key: Option<String>,
}

#[given(regex = r#"^a configuration file with token \"(.+)\"$"#)]
fn config_file_with_token(world: &mut ConfigWorld, token: String) {
    let dir = TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, format!("github_token='{token}'")).expect("write file");
    world.dir = Some(dir);
    world.path = Some(path);
    unsafe {
        std::env::remove_var("COMENQD_SOCKET_PATH");
    }
}

#[given("an invalid configuration file")]
fn invalid_configuration_file(world: &mut ConfigWorld) {
    let dir = TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "github_token='abc' this is not toml").expect("write file");
    world.dir = Some(dir);
    world.path = Some(path);
}

#[given("a configuration file without github_token")]
fn config_file_without_token(world: &mut ConfigWorld) {
    let dir = TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "socket_path='/tmp/s.sock'").expect("write file");
    world.dir = Some(dir);
    world.path = Some(path);
}

#[given(regex = r#"^a configuration file with token \"(.+)\" and no socket_path$"#)]
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
    unsafe {
        std::env::remove_var("COMENQD_SOCKET_PATH");
    }
}

#[given("a missing configuration file")]
fn missing_configuration_file(world: &mut ConfigWorld) {
    world.path = Some(PathBuf::from("/nonexistent/nowhere.toml"));
}

#[given(regex = r#"^environment variable \"(.+)\" is \"(.+)\"$"#)]
fn set_env_var(world: &mut ConfigWorld, key: String, value: String) {
    unsafe {
        std::env::set_var(&key, &value);
    }
    world.env_key = Some(key);
}

#[when("the config is loaded")]
fn load_config(world: &mut ConfigWorld) {
    let path = world.path.as_ref().expect("path set");
    world.result = Some(Config::from_file(path));
}

#[then(regex = r#"^github token is \"(.+)\"$"#)]
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

impl Drop for ConfigWorld {
    fn drop(&mut self) {
        if let Some(ref key) = self.env_key {
            unsafe {
                std::env::remove_var(key);
            }
        }
    }
}
