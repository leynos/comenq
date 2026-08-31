//! CLI parsing and command-conversion tests.

use super::{Args, Command, RepoSlug, RepoSlugParseError};
use clap::Parser;
use rstest::rstest;
use std::path::PathBuf;
use test_support::EnvVarGuard;

#[rstest]
#[case("octocat/hello-world", 1, "Hi")]
fn parses_valid_put_arguments(#[case] slug: &str, #[case] pr: u64, #[case] body: &str) {
    let pr_str = pr.to_string();
    let args = Args::try_parse_from(["comenq", "put", slug, &pr_str, body]);
    let args = args.expect("valid arguments should parse");
    let expected: RepoSlug = slug.parse().expect("slug parses");
    let Command::Put {
        repo_slug,
        pr_number,
        comment_body,
        now,
    } = args.command
    else {
        panic!("expected put command");
    };
    assert_eq!(repo_slug, expected);
    assert_eq!(pr_number, pr);
    assert_eq!(comment_body, body);
    assert!(!now, "put must default to deferred posting");
}

#[test]
fn put_accepts_the_now_flag() {
    let args = Args::try_parse_from(["comenq", "put", "--now", "octocat/hello-world", "1", "Hi"])
        .expect("valid arguments should parse");
    let Command::Put { now, .. } = args.command else {
        panic!("expected put command");
    };
    assert!(now);
    let comenq_lib::protocol::Request::Put { immediate, .. } = args.command.to_request() else {
        panic!("expected put request");
    };
    assert!(immediate);
}

#[rstest]
#[case::list(&["comenq", "list"])]
#[case::bump(&["comenq", "bump", "1a2b3c4d"])]
#[case::bust(&["comenq", "bust", "1a2b3c4d"])]
#[case::del(&["comenq", "del", "1a2b3c4d"])]
fn parses_queue_management_subcommands(#[case] argv: &[&str]) {
    let args = Args::try_parse_from(argv).expect("valid arguments should parse");
    match (argv[1], args.command) {
        ("list", Command::List) => {}
        ("bump", Command::Bump { id })
        | ("bust", Command::Bust { id })
        | ("del", Command::Del { id }) => assert_eq!(id, "1a2b3c4d"),
        (name, other) => panic!("unexpected parse for {name}: {other:?}"),
    }
}

#[test]
fn missing_subcommand_is_rejected() {
    assert!(Args::try_parse_from(["comenq"]).is_err());
}

#[rstest]
#[case("octocat")]
#[case("/repo")]
#[case("owner/")]
#[case("owner/repo/extra")]
fn rejects_invalid_slug(#[case] slug: &str) {
    let result = Args::try_parse_from(["comenq", "put", slug, "1", "Hi"]);
    let err = result.expect_err("invalid slug should be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("invalid repository format, use 'owner/repo'"),
        "unexpected error: {msg}"
    );
}

#[rstest]
#[case("octocat", RepoSlugParseError::MissingSlash)]
#[case("/repo", RepoSlugParseError::EmptyOwner)]
#[case("owner/", RepoSlugParseError::EmptyRepo)]
#[case("owner/repo/extra", RepoSlugParseError::ExtraSlashes)]
fn from_str_rejects_invalid_inputs(#[case] input: &str, #[case] expected: RepoSlugParseError) {
    let err = input
        .parse::<RepoSlug>()
        .expect_err("invalid slug should fail");
    assert_eq!(err, expected);
}

#[test]
fn display_round_trips() {
    let slug: RepoSlug = "octocat/hello".parse().expect("slug parses");
    assert_eq!(slug.to_string(), "octocat/hello");
}

#[test]
fn trims_whitespace() {
    let slug: RepoSlug = "  octocat/hello-world  ".parse().expect("slug parses");
    assert_eq!(slug.owner(), "octocat");
    assert_eq!(slug.repo(), "hello-world");
}

#[serial_test::serial]
#[test]
fn socket_defaults_to_system_path_without_runtime_dir() {
    let _socket_guard = EnvVarGuard::remove("COMENQ_SOCKET");
    let _xdg_guard = EnvVarGuard::remove("XDG_RUNTIME_DIR");
    let args = Args::try_parse_from(["comenq", "put", "octocat/hello-world", "1", "Hi"])
        .expect("valid arguments should parse");
    assert_eq!(args.socket, None);
    assert_eq!(
        args.socket_candidates(),
        vec![PathBuf::from(comenq_transport::DEFAULT_SOCKET_PATH)]
    );
}

#[serial_test::serial]
#[test]
fn socket_candidates_prefer_the_user_runtime_path() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let _socket_guard = EnvVarGuard::remove("COMENQ_SOCKET");
    let _xdg_guard = EnvVarGuard::set(
        "XDG_RUNTIME_DIR",
        dir.path().to_str().expect("tempdir path is UTF-8"),
    );
    let args = Args::try_parse_from(["comenq", "put", "octocat/hello-world", "1", "Hi"])
        .expect("valid arguments should parse");
    assert_eq!(args.socket, None);
    assert_eq!(
        args.socket_candidates(),
        vec![
            dir.path().join("comenq/comenq.sock"),
            PathBuf::from(comenq_transport::DEFAULT_SOCKET_PATH),
        ]
    );
}

#[serial_test::serial]
#[test]
fn socket_env_var_overrides_default() {
    let _socket_guard = EnvVarGuard::set("COMENQ_SOCKET", "/tmp/custom.sock");
    let args = Args::try_parse_from(["comenq", "list"]).expect("valid arguments should parse");
    assert_eq!(args.socket, Some(PathBuf::from("/tmp/custom.sock")));
    assert_eq!(
        args.socket_candidates(),
        vec![PathBuf::from("/tmp/custom.sock")]
    );
}

#[serial_test::serial]
#[test]
fn socket_flag_overrides_env_var() {
    let _socket_guard = EnvVarGuard::set("COMENQ_SOCKET", "/tmp/env.sock");
    let args = Args::try_parse_from([
        "comenq",
        "put",
        "octocat/hello-world",
        "1",
        "Hi",
        "--socket",
        "/tmp/flag.sock",
    ])
    .expect("valid arguments should parse");
    assert_eq!(args.socket, Some(PathBuf::from("/tmp/flag.sock")));
}
