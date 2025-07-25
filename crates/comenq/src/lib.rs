//! Library utilities for the `comenq` CLI.

use clap::Parser;
use comenq_lib::CommentRequest;
use std::path::PathBuf;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

/// Command line arguments for the `comenq` client.
#[derive(Debug, Clone, Parser)]
#[command(name = "comenq", about = "Enqueue a GitHub PR comment")]
pub struct Args {
    /// The repository in 'owner/repo' format (e.g., "rust-lang/rust").
    #[arg(value_parser = validate_repo_slug)]
    pub repo_slug: String,

    /// The pull request number to comment on.
    pub pr_number: u64,

    /// The body of the comment. It is recommended to quote this argument.
    pub comment_body: String,

    /// Path to the daemon's Unix Domain Socket.
    #[arg(long, default_value = "/run/comenq/socket")]
    pub socket: PathBuf,
}

fn validate_repo_slug(s: &str) -> Result<String, String> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Ok(s.to_owned())
    } else {
        Err(String::from("invalid repository format, use 'owner/repo'"))
    }
}

/// Errors that can occur when interacting with the daemon.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Connecting to the daemon failed.
    #[error("failed to connect to daemon: {0}")]
    Connect(#[from] std::io::Error),
    /// Serialising the request failed.
    #[error("failed to serialise request: {0}")]
    Serialise(#[from] serde_json::Error),
    /// Writing the request to the socket failed.
    #[error("failed to write to daemon: {0}")]
    Write(#[source] std::io::Error),
}

/// Send a `CommentRequest` to the daemon.
///
/// # Examples
///
/// ```no_run
/// # use comenq::{Args, run};
/// # use std::path::PathBuf;
/// # async fn try_run() -> Result<(), comenq::ClientError> {
/// let args = Args {
///     repo_slug: "owner/repo".into(),
///     pr_number: 1,
///     comment_body: String::from("Hi"),
///     socket: PathBuf::from("/run/comenq/socket"),
/// };
/// run(args).await?;
/// # Ok(())
/// # }
/// ```
pub async fn run(args: Args) -> Result<(), ClientError> {
    let (owner, repo) = parse_slug(&args.repo_slug);
    let request = CommentRequest {
        owner,
        repo,
        pr_number: args.pr_number,
        body: args.comment_body,
    };

    let payload = serde_json::to_vec(&request)?;

    let mut stream = UnixStream::connect(&args.socket)
        .await
        .map_err(ClientError::Connect)?;
    stream
        .write_all(&payload)
        .await
        .map_err(ClientError::Write)?;
    let _ = stream.shutdown().await;
    Ok(())
}

fn parse_slug(slug: &str) -> (String, String) {
    let mut parts = slug.splitn(2, '/');
    let owner = parts.next().unwrap_or_default().to_owned();
    let repo = parts.next().unwrap_or_default().to_owned();
    (owner, repo)
}

#[cfg(test)]
mod tests {
    use super::{Args, ClientError, run};
    use clap::Parser;
    use comenq_lib::CommentRequest;
    use rstest::rstest;
    use tempfile::tempdir;
    use tokio::io::AsyncReadExt;
    use tokio::net::UnixListener;

    #[rstest]
    #[case("octocat/hello-world", 1, "Hi")]
    fn parses_valid_arguments(#[case] slug: &str, #[case] pr: u64, #[case] body: &str) {
        let pr_str = pr.to_string();
        let args = Args::try_parse_from(["comenq", slug, &pr_str, body]);
        let args = args.expect("valid arguments should parse");
        assert_eq!(args.repo_slug, slug);
        assert_eq!(args.pr_number, pr);
        assert_eq!(args.comment_body, body);
    }

    #[rstest]
    #[case("octocat")]
    #[case("/repo")]
    #[case("owner/")]
    #[case("owner/repo/extra")]
    fn rejects_invalid_slug(#[case] slug: &str) {
        let result = Args::try_parse_from(["comenq", slug, "1", "Hi"]);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn run_sends_request() {
        let dir = tempdir().expect("temp dir");
        let socket = dir.path().join("sock");
        let listener = UnixListener::bind(&socket).expect("bind socket");

        let accept = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.expect("read");
            serde_json::from_slice::<CommentRequest>(&buf).expect("deserialize")
        });

        let args = Args {
            repo_slug: "octocat/hello-world".into(),
            pr_number: 1,
            comment_body: "Hi".into(),
            socket: socket.clone(),
        };

        run(args).await.expect("run succeeds");
        let req = accept.await.expect("join");
        assert_eq!(req.owner, "octocat");
        assert_eq!(req.repo, "hello-world");
        assert_eq!(req.pr_number, 1);
        assert_eq!(req.body, "Hi");
    }

    #[tokio::test]
    async fn run_errors_when_socket_missing() {
        let dir = tempdir().expect("temp dir");
        let socket = dir.path().join("nosock");

        let args = Args {
            repo_slug: "octocat/hello-world".into(),
            pr_number: 1,
            comment_body: "Hi".into(),
            socket: socket.clone(),
        };

        let err = run(args).await.expect_err("should error");
        assert!(matches!(err, ClientError::Connect(_)));
    }
}
