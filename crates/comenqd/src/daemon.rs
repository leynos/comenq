//! Asynchronous daemon tasks for comenqd.
//!
//! This module provides the run function used by `main` which spawns the
//! Unix socket listener and the queue worker.

use crate::config::Config;
use anyhow::Result;
use comenq_lib::CommentRequest;
use octocrab::Octocrab;
use std::fs as stdfs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::net::{UnixListener, UnixStream};
use yaque::{Receiver, Sender, channel};

fn build_octocrab(token: &str) -> Result<Octocrab> {
    Ok(Octocrab::builder()
        .personal_token(token.to_string())
        .build()?)
}

fn prepare_listener(path: &Path) -> Result<UnixListener> {
    if stdfs::metadata(path).is_ok() {
        stdfs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    stdfs::set_permissions(path, stdfs::Permissions::from_mode(0o660))?;
    Ok(listener)
}

async fn ensure_queue_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).await?;
    Ok(())
}

/// Start the daemon with the provided configuration.
pub async fn run(config: Config) -> Result<()> {
    ensure_queue_dir(&config.queue_path).await?;
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
    async fn ensure_queue_dir_creates_directory() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("queue");
        ensure_queue_dir(&path).await.unwrap();
        assert!(path.is_dir());
    }
}
