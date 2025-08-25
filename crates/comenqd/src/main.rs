//! Entry point for the Comenqd daemon binary.
//! Spawns the background service that processes `CommentRequest`s received
//! from the CLI client and coordinates persistence.

use tracing::info;

mod logging;

mod config;
mod daemon;
mod listener;
mod supervisor;
mod worker;
use config::Config;
use daemon::run;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logging::init();
    let cfg = Config::load()?;
    info!(socket = ?cfg.socket_path, queue = ?cfg.queue_path, "Comenqd daemon started");
    run(cfg).await
}
