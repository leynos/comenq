//! Library utilities for the `comenq` CLI.

use clap::{Parser, Subcommand, builder::ValueHint};
use std::{fmt, path::PathBuf, str::FromStr};
use thiserror::Error;

mod client;
mod output;

pub use client::{ClientError, run};
pub use output::{format_eta, one_line_summary};

/// A GitHub repository slug in `owner/repo` format.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoSlug {
    /// Repository owner.
    owner: String,
    /// Repository name.
    repo: String,
}

impl RepoSlug {
    /// Repository owner.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use comenq::RepoSlug;
    /// let slug: RepoSlug = "octocat/hello-world".parse().expect("slug parses");
    /// assert_eq!(slug.owner(), "octocat");
    /// ```
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Repository name.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use comenq::RepoSlug;
    /// let slug: RepoSlug = "octocat/hello-world".parse().expect("slug parses");
    /// assert_eq!(slug.repo(), "hello-world");
    /// ```
    pub fn repo(&self) -> &str {
        &self.repo
    }
}

/// Error returned when parsing a [`RepoSlug`] fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepoSlugParseError {
    /// Missing slash separator.
    #[error("invalid repository format, use 'owner/repo'")]
    MissingSlash,
    /// Owner segment is empty.
    #[error("invalid repository format, use 'owner/repo'")]
    EmptyOwner,
    /// Repository segment is empty.
    #[error("invalid repository format, use 'owner/repo'")]
    EmptyRepo,
    /// Extra slash found in repository segment.
    #[error("invalid repository format, use 'owner/repo'")]
    ExtraSlashes,
}

impl FromStr for RepoSlug {
    type Err = RepoSlugParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (owner, repo) = s.split_once('/').ok_or(RepoSlugParseError::MissingSlash)?;
        if owner.is_empty() {
            return Err(RepoSlugParseError::EmptyOwner);
        }
        if repo.is_empty() {
            return Err(RepoSlugParseError::EmptyRepo);
        }
        if repo.contains('/') {
            return Err(RepoSlugParseError::ExtraSlashes);
        }
        Ok(Self {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
        })
    }
}

impl fmt::Display for RepoSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

/// Command line arguments for the `comenq` client.
#[derive(Debug, Clone, Parser)]
#[command(name = "comenq", about = "Queue and manage GitHub PR comments")]
pub struct Args {
    /// Path to the daemon's Unix Domain Socket.
    ///
    /// When omitted, the client tries the per-user runtime path
    /// (`$XDG_RUNTIME_DIR/comenq/comenq.sock`) and then the system path,
    /// connecting to the first socket that accepts, so a user-hosted daemon
    /// is found automatically and a stale socket file never shadows a
    /// healthy daemon. May be overridden with the `COMENQ_SOCKET`
    /// environment variable or this flag.
    // The candidates are resolved at connect time rather than through
    // clap's `default_value_os_t`, which caches the computed value in a
    // process-wide static and would ignore later environment changes.
    #[arg(long, global = true, value_hint = ValueHint::FilePath, env = "COMENQ_SOCKET")]
    pub socket: Option<PathBuf>,

    /// Queue operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Queue operations offered by the client.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Enqueue a comment and print its identifier and approximate ETA.
    ///
    /// By default the comment waits one full cooldown (plus its flutter)
    /// from enqueue even when the queue is idle; pass `--now` to post as
    /// soon as the queue allows.
    Put {
        /// The repository in 'owner/repo' format (e.g., "rust-lang/rust").
        repo_slug: RepoSlug,

        /// The pull request number to comment on.
        pr_number: u64,

        /// The body of the comment. It is recommended to quote this argument.
        comment_body: String,

        /// Post as soon as the queue allows instead of waiting a full
        /// cooldown from enqueue.
        #[arg(long)]
        now: bool,
    },
    /// List pending comments with identifiers and ETAs.
    List,
    /// Move the identified comment to the head of the queue.
    Bump {
        /// Identifier printed by `put` and `list`.
        id: String,
    },
    /// Move the identified comment to the tail of the queue.
    Bust {
        /// Identifier printed by `put` and `list`.
        id: String,
    },
    /// Remove the identified comment from the queue.
    Del {
        /// Identifier printed by `put` and `list`.
        id: String,
    },
}

impl Command {
    /// The protocol request this command performs.
    #[must_use]
    pub fn to_request(&self) -> comenq_lib::protocol::Request {
        use comenq_lib::protocol::Request;
        match self {
            Self::Put {
                repo_slug,
                pr_number,
                comment_body,
                now,
            } => Request::Put {
                request: comenq_lib::CommentRequest {
                    owner: repo_slug.owner().to_owned(),
                    repo: repo_slug.repo().to_owned(),
                    pr_number: *pr_number,
                    body: comment_body.clone(),
                },
                immediate: *now,
            },
            Self::List => Request::List,
            Self::Bump { id } => Request::Bump { id: id.clone() },
            Self::Bust { id } => Request::Bust { id: id.clone() },
            Self::Del { id } => Request::Del { id: id.clone() },
        }
    }
}

impl Args {
    /// Socket paths to try in order, honouring an explicit override.
    ///
    /// An explicit `--socket` (or `COMENQ_SOCKET`) yields exactly that
    /// path; otherwise the discovery candidates from
    /// [`comenq_lib::socket_candidates`] are returned. Callers connect to
    /// each in turn so a stale socket file cannot shadow a live daemon.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use clap::Parser;
    /// use comenq::Args;
    ///
    /// let args = Args::try_parse_from([
    ///     "comenq",
    ///     "put",
    ///     "octocat/hello-world",
    ///     "1",
    ///     "Hi",
    ///     "--socket",
    ///     "/tmp/comenq.sock",
    /// ])
    /// .expect("arguments parse");
    /// assert_eq!(
    ///     args.socket_candidates(),
    ///     vec![std::path::PathBuf::from("/tmp/comenq.sock")]
    /// );
    /// ```
    #[must_use]
    pub fn socket_candidates(&self) -> Vec<PathBuf> {
        self.socket
            .clone()
            .map_or_else(comenq_lib::socket_candidates, |explicit| vec![explicit])
    }
}

#[cfg(test)]
mod tests {
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
        let args =
            Args::try_parse_from(["comenq", "put", "--now", "octocat/hello-world", "1", "Hi"])
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
        // Ensure the CLI surfaces the canonical repo format error.
        // This guards regressions in the Display of the parse error
        // as rendered through clap's error handling.
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
            vec![PathBuf::from(comenq_lib::DEFAULT_SOCKET_PATH)]
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
                PathBuf::from(comenq_lib::DEFAULT_SOCKET_PATH),
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
}
