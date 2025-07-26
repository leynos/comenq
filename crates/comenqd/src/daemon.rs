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
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::net::{UnixListener, UnixStream};
use yaque::{Receiver, Sender, channel};

/// Start the daemon with the provided configuration.
pub async fn run(config: Config) -> Result<()> {
    let octocrab = Arc::new(
        Octocrab::builder()
            .personal_token(config.github_token.clone())
            .build()?,
    );
    let (tx, rx) = channel(&config.queue_path)?;
    let cfg = Arc::new(config);

    let listener = tokio::spawn(run_listener(cfg.clone(), tx));
    let worker = tokio::spawn(run_worker(cfg.clone(), rx, octocrab));

    tokio::select! {
        res = listener => res??,
        res = worker => res??,
    }

    Ok(())
}

async fn run_listener(config: Arc<Config>, mut tx: Sender) -> Result<()> {
    if fs::metadata(&config.socket_path).is_ok() {
        fs::remove_file(&config.socket_path)?;
    }
    let listener = UnixListener::bind(&config.socket_path)?;
    fs::set_permissions(&config.socket_path, fs::Permissions::from_mode(0o660))?;

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

async fn run_worker(
    config: Arc<Config>,
    mut rx: Receiver,
    octocrab: Arc<Octocrab>,
) -> Result<()> {
    loop {
        let guard = rx.recv().await?;
        let request: CommentRequest = serde_json::from_slice(&guard)?;
        let result = octocrab
            .issues(&request.owner, &request.repo)
            .create_comment(request.pr_number, &request.body)
            .await;

        if result.is_ok() {
            guard.commit()?;
        } else {
            tracing::error!(?result, "Failed to post comment");
        }

        tokio::time::sleep(std::time::Duration::from_secs(
            config.cooldown_period_seconds,
        ))
        .await;
    }
}
