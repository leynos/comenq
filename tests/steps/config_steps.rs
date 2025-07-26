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
}

#[given(regex = r#"^a configuration file with token \"(.+)\"$"#)]
fn config_file_with_token(world: &mut ConfigWorld, token: String) {
    let dir = TempDir::new().expect("create temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, format!("github_token='{token}'")).expect("write file");
    world.dir = Some(dir);
    world.path = Some(path);
}

#[given("a missing configuration file")]
fn missing_configuration_file(world: &mut ConfigWorld) {
    world.path = Some(PathBuf::from("/nonexistent/nowhere.toml"));
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
