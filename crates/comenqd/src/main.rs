//! Entry point for the Comenqd daemon binary.
//! Spawns the background service that processes `CommentRequest`s received
//! from the CLI client and coordinates persistence.

use tracing::{info, warn};

use color_eyre::eyre::Context;
use comenqd::{config::Config, daemon};

mod logging;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    logging::init();
    color_eyre::install()?;
    if let Err(error) = comenqd::metrics::install_prometheus() {
        warn!(error = %error, "Prometheus metrics exporter unavailable");
    }
    let cfg = tokio::task::spawn_blocking(Config::load)
        .await
        .context("configuration loading task failed")??;
    info!(
        socket = ?cfg.socket_path,
        queue = ?cfg.queue_path,
        "Comenqd daemon started"
    );
    daemon::run(cfg)
        .await
        .context("daemon exited unexpectedly")?;
    Ok(())
}
