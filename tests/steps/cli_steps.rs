#![allow(clippy::expect_used, reason = "simplify test failure output")]

use clap::Parser;
use cucumber::{World, given, then, when};
use std::ffi::OsString;

use comenq::Args;

#[derive(Debug, Default, World)]
pub struct CliWorld {
    args: Option<Vec<OsString>>,
    result: Option<Result<Args, clap::Error>>,
}

#[given("valid CLI arguments")]
fn valid_cli_arguments(world: &mut CliWorld) {
    world.args = Some(vec![
        OsString::from("comenq"),
        OsString::from("octocat/hello-world"),
        OsString::from("1"),
        OsString::from("Hi"),
    ]);
}

#[given(regex = r#"^CLI arguments with repo slug \"(.+)\"$"#)]
fn cli_args_with_repo_slug(world: &mut CliWorld, slug: String) {
    world.args = Some(vec![
        OsString::from("comenq"),
        OsString::from(slug),
        OsString::from("1"),
        OsString::from("Hi"),
    ]);
}

#[when("they are parsed")]
fn they_are_parsed(world: &mut CliWorld) {
    if let Some(args) = world.args.clone() {
        world.result = Some(Args::try_parse_from(args));
    }
}

#[then("parsing succeeds")]
fn parsing_succeeds(world: &mut CliWorld) {
    match world.result.take() {
        Some(Ok(_)) => {}
        other => panic!("expected success, got {other:?}"),
    }
}

#[then("an error is returned")]
fn an_error_is_returned(world: &mut CliWorld) {
    match world.result.take() {
        Some(Err(_)) => {}
        other => panic!("expected error, got {other:?}"),
    }
}
