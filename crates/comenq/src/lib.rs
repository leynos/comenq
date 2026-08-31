//! Client argument parsing, protocol requests, and safe human-readable output.
//!
//! [`Args`] and [`Command`] describe the `put`, `list`, `bump`, `bust`, and
//! `del` operations. [`run`] sends each command to the daemon as one tagged
//! request and returns [`ClientError`] for transport, timeout, protocol, or
//! daemon failures.

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
    /// When omitted and `XDG_RUNTIME_DIR` is set to a non-empty absolute path,
    /// the client tries the per-user runtime path
    /// (`$XDG_RUNTIME_DIR/comenq/comenq.sock`) and then the system path,
    /// connecting to the first socket that accepts. Otherwise, it falls back
    /// to the system socket. This finds a user-hosted daemon automatically
    /// without letting a stale socket file shadow a healthy daemon. May be
    /// overridden with the `COMENQ_SOCKET` environment variable or this flag.
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
    /// [`comenq_transport::socket_candidates`] are returned. Callers connect to
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
            .map_or_else(comenq_transport::socket_candidates, |explicit| {
                vec![explicit]
            })
    }
}

#[cfg(test)]
mod tests;
