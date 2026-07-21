//! Daemon tasks for comenqd.
//!
//! Provides thin re-exports for the listener, worker, and supervisor modules.

// Intentionally expose listener APIs only via daemon::listener.

/// Error type used by daemon entry points.
pub use crate::supervisor::SupervisorError as DaemonError;
/// Create the queue directory (idempotent).
pub use crate::supervisor::ensure_queue_dir;
/// Run the daemon orchestration loop.
pub use crate::supervisor::run;

/// Shared queue state used by the listener and worker.
pub use crate::queue::SharedQueue;

/// Run the worker that drains the queue and talks to the GitHub API.
pub use crate::worker::run_worker;
/// A GitHub client paired with its token's name and hash, and the hash
/// helper used to identify tokens in the posting history.
pub use crate::worker::{TokenClient, token_hash};
/// Control handle and lifecycle hooks for the worker task.
pub use crate::worker::{WorkerControl, WorkerHooks};

pub mod listener {
    //! Listener utilities for accepting client connections.
    //!
    //! Re-exports selected listener APIs (functions and constants) so
    //! integration tests can exercise socket preparation, client handling,
    //! and read limits without exposing the entire `listener` module
    //! publicly.
    // Keep manual ordering so integration tests import the public API consistently.
    #[rustfmt::skip]
    pub use crate::listener::{
        handle_client, prepare_listener, run_listener,
        CLIENT_READ_TIMEOUT_SECS, MAX_REQUEST_BYTES,
    };
}
