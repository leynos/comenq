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

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use crate::util::is_metadata_file;

/// Listener utilities for accepting client connections.
///
/// Re-exports selected listener APIs (functions and constants) so integration
/// tests can exercise socket preparation, client handling, and read limits
/// without exposing the entire `listener` module publicly.
pub mod listener {
    // Keep manual ordering so integration tests import the public API consistently.
    #[rustfmt::skip]
    pub use crate::listener::{
        handle_client, prepare_listener, run_listener,
        CLIENT_READ_TIMEOUT_SECS, MAX_REQUEST_BYTES,
    };
}
