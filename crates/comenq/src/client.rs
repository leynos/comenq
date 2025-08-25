//! Client-side communication with the `comenqd` daemon.
//!
//! This module contains the logic to serialize a comment request and send it to
//! the daemon over its Unix Domain Socket. It is separated from `lib.rs` so
//! that argument parsing remains focused and the network logic is easily
//! testable.

use comenq_lib::CommentRequest;
use thiserror::Error;
use tokio::{io::AsyncWriteExt, net::UnixStream};
use tracing::warn;

use crate::Args;

/// Errors that can occur when interacting with the daemon.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Connecting to the daemon failed.
    #[error("failed to connect to daemon: {0}")]
    Connect(#[from] std::io::Error),
    /// Serializing the request failed.
    #[error("failed to serialize request: {0}")]
    Serialize(#[from] serde_json::Error),
    /// The repository slug was invalid.
    #[error("invalid repository format")]
    BadSlug,
    /// Writing the request to the socket failed.
    #[error("failed to write to daemon: {0}")]
    Write(#[source] std::io::Error),
    /// Shutting down the socket failed.
    #[error("failed to close connection: {0}")]
    Shutdown(#[source] std::io::Error),
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
///     socket: PathBuf::from("/run/comenq/comenq.sock"),
/// };
/// run(args).await?;
/// # Ok(())
/// # }
/// ```
pub async fn run(args: Args) -> Result<(), ClientError> {
    if crate::validate_repo_slug(&args.repo_slug).is_err() {
        return Err(ClientError::BadSlug);
    }
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
    if let Err(e) = stream.shutdown().await {
        warn!("failed to close connection: {e}");
        return Err(ClientError::Shutdown(e));
    }
    Ok(())
}

fn parse_slug(slug: &str) -> (String, String) {
    // safe expect: `validate_repo_slug` ensures two non-empty parts
    let (owner, repo) = slug
        .split_once('/')
        .expect("slug should have been validated by validate_repo_slug");
    (owner.to_owned(), repo.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{ClientError, parse_slug, run};
    use crate::Args;
    use comenq_lib::CommentRequest;
    use tempfile::tempdir;
    use tokio::io::AsyncReadExt;
    use tokio::net::UnixListener;

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

    #[tokio::test]
    async fn run_errors_on_bad_slug() {
        let dir = tempdir().expect("temp dir");
        let socket = dir.path().join("sock");
        let _listener = UnixListener::bind(&socket).expect("bind socket");

        let args = Args {
            repo_slug: "badslug".into(),
            pr_number: 1,
            comment_body: "Hi".into(),
            socket,
        };

        let err = run(args).await.expect_err("should error");
        assert!(matches!(err, ClientError::BadSlug));
    }

    #[test]
    fn slug_is_split() {
        let (owner, repo) = parse_slug("octocat/hello-world");
        assert_eq!(owner, "octocat");
        assert_eq!(repo, "hello-world");
    }
}
