//! Behavioural test steps for the CLI argument parser.
//!
//! These steps drive the Cucumber scenarios that verify valid and
//! invalid command line inputs, including the optional `--socket`
//! flag. They ensure the parser surface behaves as documented.

use clap::Parser;
use cucumber::{World, given, then, when};
use std::ffi::OsString;
use std::path::PathBuf;

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

#[given("no CLI arguments")]
fn no_cli_arguments(world: &mut CliWorld) {
    world.args = Some(vec![OsString::from("comenq")]);
}

#[given(regex = r#"^socket path \"(.+)\"$"#)]
fn socket_path(world: &mut CliWorld, path: String) {
    if let Some(mut args) = world.args.take() {
        args.push(OsString::from("--socket"));
        args.push(OsString::from(path));
        world.args = Some(args);
    }
}

#[when("they are parsed")]
#[expect(clippy::expect_used, reason = "simplify test failure output")]
fn they_are_parsed(world: &mut CliWorld) {
    let args = world
        .args
        .clone()
        .expect("world.args should be set by a given step");
    world.result = Some(Args::try_parse_from(args));
}

#[then("parsing succeeds")]
fn parsing_succeeds(world: &mut CliWorld) {
    match world.result.as_ref() {
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

#[then(regex = r#"^the socket path is \"(.+)\"$"#)]
fn the_socket_path_is(world: &mut CliWorld, expected: String) {
    let args = match world.result.take() {
        Some(Ok(a)) => a,
        other => panic!("expected parsed args, got {other:?}"),
    };
    assert_eq!(args.socket, PathBuf::from(expected));
}
