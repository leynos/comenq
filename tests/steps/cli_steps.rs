//! Behavioural test steps for the CLI argument parser.
//!
//! These steps drive scenarios that verify valid and invalid
//! command line inputs, including the optional `--socket` flag.
//! They ensure the parser surface behaves as documented.
#![expect(clippy::expect_used, reason = "simplify test failure output")]

use clap::Parser;
use rstest_bdd_macros::{given, then, when};
use std::cell::RefCell;
use std::ffi::OsString;
use std::path::PathBuf;

use comenq::Args;

#[derive(Default)]
struct CliState {
    args: Option<Vec<OsString>>,
    result: Option<Result<Args, clap::Error>>,
}

thread_local! {
    static STATE: RefCell<CliState> = RefCell::new(CliState::default());
}

#[given("valid CLI arguments")]
fn valid_cli_arguments() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.args = Some(vec![
            OsString::from("comenq"),
            OsString::from("octocat/hello-world"),
            OsString::from("1"),
            OsString::from("Hi"),
        ]);
    });
}

#[given("CLI arguments with repo slug \"{slug}\"")]
fn cli_args_with_repo_slug(slug: String) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.args = Some(vec![
            OsString::from("comenq"),
            OsString::from(slug),
            OsString::from("1"),
            OsString::from("Hi"),
        ]);
    });
}

#[given("no CLI arguments")]
fn no_cli_arguments() {
    STATE.with(|s| {
        s.borrow_mut().args = Some(vec![OsString::from("comenq")]);
    });
}

#[given("socket path \"{path}\"")]
fn socket_path(path: String) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if let Some(args) = st.args.as_mut() {
            args.push(OsString::from("--socket"));
            args.push(OsString::from(path));
        }
    });
}

#[when("they are parsed")]
fn they_are_parsed() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let args = st.args.clone().expect("args should be set by a given step");
        st.result = Some(Args::try_parse_from(args));
    });
}

#[then("parsing succeeds")]
fn parsing_succeeds() {
    STATE.with(|s| match s.borrow().result.as_ref() {
        Some(Ok(_)) => {}
        other => panic!("expected success, got {other:?}"),
    });
}

#[then("an error is returned")]
fn an_error_is_returned() {
    STATE.with(|s| match s.borrow().result.as_ref() {
        Some(Err(_)) => {}
        other => panic!("expected error, got {other:?}"),
    });
}

#[then("the socket path is \"{expected}\"")]
fn the_socket_path_is(expected: String) {
    STATE.with(|s| {
        let st = s.borrow();
        let args = match st.result.as_ref() {
            Some(Ok(a)) => a,
            other => panic!("expected parsed args, got {other:?}"),
        };
        assert_eq!(args.socket, PathBuf::from(expected));
    });
}
