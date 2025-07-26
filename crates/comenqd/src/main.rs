//! Entry point for the Comenqd daemon binary.
//! Spawns the background service that processes `CommentRequest`s received
//! from the CLI client and coordinates persistence.

use tracing::info;

mod config;
use config::Config;

fn main() -> Result<(), ortho_config::OrthoError> {
    tracing_subscriber::fmt::init();
    let cfg = Config::load()?;
    info!(socket = ?cfg.socket_path, queue = ?cfg.queue_path, "Comenqd daemon started");
    Ok(())
}
