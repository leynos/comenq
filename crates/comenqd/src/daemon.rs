//! Asynchronous daemon tasks for comenqd.
//!
//! This module provides the run function used by `main` which spawns the
//! Unix socket listener and the queue worker.

use crate::config::Config;
use anyhow::Result;
use comenq_lib::CommentRequest;
use octocrab::Octocrab;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::{UnixListener, UnixStream};
use yaque::{Receiver, Sender, channel};

fn build_octocrab(token: &str) -> Result<Octocrab> {
    Ok(Octocrab::builder()
        .personal_token(token.to_string())
        .build()?)
}

fn prepare_listener(path: &Path) -> Result<UnixListener> {
    if fs::metadata(path).is_ok() {
        fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))?;
    Ok(listener)
}

/// Start the daemon with the provided configuration.
pub async fn run(config: Config) -> Result<()> {
    fs::create_dir_all(&config.queue_path)?;
    tracing::info!(queue = %config.queue_path.display(), "Queue directory prepared");
    let octocrab = Arc::new(build_octocrab(&config.github_token)?);
    let (tx, rx) = channel(&config.queue_path)?;
    let cfg = Arc::new(config);

    let listener = tokio::spawn(run_listener(cfg.clone(), tx));
    let worker = tokio::spawn(run_worker(cfg.clone(), rx, octocrab));

    tokio::select! {
        res = listener => match res {
            Ok(inner) => inner?,
            Err(e) => return Err(e.into()),
        },
        res = worker => match res {
            Ok(inner) => inner?,
            Err(e) => return Err(e.into()),
        },
    }

    Ok(())
}

async fn run_listener(config: Arc<Config>, mut tx: Sender) -> Result<()> {
    let listener = prepare_listener(&config.socket_path)?;

    loop {
        let (stream, _) = listener.accept().await?;
        if let Err(e) = handle_client(stream, &mut tx).await {
            tracing::warn!(error = %e, "Client handling failed");
        }
    }
}

async fn handle_client(mut stream: UnixStream, tx: &mut Sender) -> Result<()> {
    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).await?;
    let request: CommentRequest = serde_json::from_slice(&buffer)?;
    let bytes = serde_json::to_vec(&request)?;
    tx.send(bytes).await?;
    Ok(())
}

async fn run_worker(config: Arc<Config>, mut rx: Receiver, octocrab: Arc<Octocrab>) -> Result<()> {
    loop {
        let guard = rx.recv().await?;
        let request: CommentRequest = serde_json::from_slice(&guard)?;

        let issues = octocrab.issues(&request.owner, &request.repo);
        let post = issues.create_comment(request.pr_number, &request.body);

        match tokio::time::timeout(Duration::from_secs(10), post).await {
            Ok(Ok(_)) => {
                guard.commit()?;
                tokio::time::sleep(Duration::from_secs(config.cooldown_period_seconds)).await;
            }
            Ok(Err(e)) => {
                tracing::error!(error = %e, owner = %request.owner, repo = %request.repo, pr = request.pr_number, "GitHub API call failed");
            }
            Err(e) => {
                tracing::error!(error = %e, "Timed out posting comment");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn run_creates_queue_dir() {
        let dir = tempdir().unwrap();
        let cfg = Config {
            github_token: "tok".into(),
            socket_path: dir.path().join("sock"),
            queue_path: dir.path().join("queue"),
            cooldown_period_seconds: 1,
        };

        let queue_dir = cfg.queue_path.clone();
        let handle = tokio::spawn(run(cfg));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(queue_dir.is_dir());
        handle.abort();
        let _ = handle.await;
    }
}
