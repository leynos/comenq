//! Daemon tasks for comenqd.
//!
//! Provides thin re-exports for the listener, worker, and supervisor modules.

pub use crate::listener::run_listener;
pub use crate::supervisor::{SupervisorError as DaemonError, ensure_queue_dir, queue_writer, run};
pub use crate::worker::{WorkerControl, WorkerHooks, run_worker};

/// Listener utilities for accepting client connections.
///
/// Re-exports functions from the internal `listener` module so integration tests
/// can exercise socket preparation and client handling without exposing the
/// entire module as part of the public API.
pub mod listener {
    pub use crate::listener::{handle_client, prepare_listener, run_listener};
}
