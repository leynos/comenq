//! Entry point for the Comenqd daemon binary.
//! Spawns the background service that processes `CommentRequest`s received
//! from the CLI client and coordinates persistence.

use tracing::info;

fn main() {
    tracing_subscriber::fmt::init();
    info!("Comenqd daemon started");
}
