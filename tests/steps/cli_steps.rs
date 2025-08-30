//! Behavioural test steps for the CLI argument parser.
//!
//! Verify valid and invalid command-line inputs, including the optional `--socket` flag.

use clap::Parser;
use rstest::fixture;
use rstest_bdd_macros::{given, then, when};
use std::cell::RefCell;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use comenq::Args;

/// Per-scenario state for CLI parsing tests.
///
/// The fixture returned by [`cli_state`] owns the argument vector and parser
/// result, ensuring each scenario starts with a clean slate and any resources
/// are dropped at scope exit.
#[derive(Default)]
pub struct CliState {
    // `rstest-bdd` fixtures are injected as shared references, so interior
    // mutability allows steps to update the arguments and parse result.
    args: RefCell<Option<Vec<OsString>>>,
    result: RefCell<Option<Result<Args, clap::Error>>>,
}

/// Provide a fresh [`CliState`] for each scenario.
#[fixture]
pub fn cli_state() -> CliState {
    CliState::default()
}

#[given("valid CLI arguments")]
fn valid_cli_arguments(#[from(cli_state)] state: &CliState) {
    *state.args.borrow_mut() = Some(vec![
        OsString::from("comenq"),
        OsString::from("octocat/hello-world"),
        OsString::from("1"),
        OsString::from("Hi"),
    ]);
}

#[given("CLI arguments with repo slug \"{slug}\"")]
fn cli_args_with_repo_slug(#[from(cli_state)] state: &CliState, slug: String) {
    *state.args.borrow_mut() = Some(vec![
        OsString::from("comenq"),
        OsString::from(slug),
        OsString::from("1"),
        OsString::from("Hi"),
    ]);
}

#[given("no CLI arguments")]
fn no_cli_arguments(#[from(cli_state)] state: &CliState) {
    *state.args.borrow_mut() = Some(vec![OsString::from("comenq")]);
}

#[given("socket path \"{path}\"")]
fn socket_path(#[from(cli_state)] state: &CliState, path: String) {
    let mut args_ref = state.args.borrow_mut();
    let Some(args) = args_ref.as_mut() else {
        panic!("args must be initialized by a Given step before setting socket path");
    };
    args.push(OsString::from("--socket"));
    args.push(OsString::from(path));
}

#[when("they are parsed")]
fn they_are_parsed(#[from(cli_state)] state: &CliState) {
    let Some(args) = state.args.borrow_mut().take() else {
        panic!("args should be set by a given step");
    };
    *state.result.borrow_mut() = Some(Args::try_parse_from(args));
}

#[then("parsing succeeds")]
fn parsing_succeeds(#[from(cli_state)] state: &CliState) {
    match state.result.borrow().as_ref() {
        Some(Ok(_)) => {}
        other => panic!("expected success, got {other:?}"),
    }
}

#[then("an error is returned")]
fn an_error_is_returned(#[from(cli_state)] state: &CliState) {
    match state.result.borrow().as_ref() {
        Some(Err(_)) => {}
        other => panic!("expected error, got {other:?}"),
    }
}

#[then("the socket path is \"{expected}\"")]
fn the_socket_path_is(#[from(cli_state)] state: &CliState, expected: PathBuf) {
    let binding = state.result.borrow();
    let args = match binding.as_ref() {
        Some(Ok(a)) => a,
        other => panic!("expected parsed args, got {other:?}"),
    };
    let expected = fs::canonicalize(&expected).unwrap_or(expected);
    let actual = fs::canonicalize(&args.socket).unwrap_or_else(|_| args.socket.clone());
    assert_eq!(
        actual,
        expected,
        "socket path mismatch: actual={} expected={}",
        actual.display(),
        expected.display(),
    );
}
