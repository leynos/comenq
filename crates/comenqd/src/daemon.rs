//! Daemon tasks for comenqd.
//!
//! Provides thin re-exports for the listener, worker, and supervisor modules.

// Intentionally expose listener APIs only via daemon::listener.

/// Error type used by daemon entry points.
pub use crate::supervisor::SupervisorError as DaemonError;
/// Create the queue directory (idempotent).
pub use crate::supervisor::ensure_queue_dir;
/// Spawn the background writer that persists enqueued payloads.
pub use crate::supervisor::queue_writer;
/// Run the daemon orchestration loop.
pub use crate::supervisor::run;

/// Run the worker that drains the queue and talks to the GitHub API.
pub use crate::worker::run_worker;
/// Control handle and lifecycle hooks for the worker task.
pub use crate::worker::{WorkerControl, WorkerHooks};

/// Listener utilities for accepting client connections.
///
/// Re-exports functions from the internal `listener` module so integration tests
/// can exercise socket preparation and client handling without exposing the
/// entire module as part of the public API.
pub mod listener {
    pub use crate::listener::{handle_client, prepare_listener, run_listener};
}
