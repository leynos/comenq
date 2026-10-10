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

/// Control handle for the worker task.
pub use crate::worker::WorkerControl;
/// Lifecycle hooks exposed to test and test-support builds.
#[cfg(any(test, feature = "test-support"))]
pub use crate::worker::WorkerHooks;
/// Run the worker that drains the queue and talks to the GitHub API.
pub use crate::worker::run_worker;

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
        CLIENT_READ_TIMEOUT_SECS, CLIENT_WRITE_TIMEOUT_SECS, MAX_REQUEST_BYTES,
    };

    /// Execute one protocol request through the listener's queue adapter.
    ///
    /// This is useful when a caller needs the daemon's request mapping without
    /// opening a Unix socket.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use comenq_lib::protocol::{Request, Response};
    /// # use comenqd::queue::SharedQueue;
    /// # async fn example(queue: &SharedQueue) {
    /// let response = comenqd::daemon::listener::dispatch_request(queue, Request::List).await;
    /// assert!(matches!(response, Response::Ok { .. }));
    /// # }
    /// ```
    pub async fn dispatch_request(
        queue: &crate::queue::SharedQueue,
        request: comenq_lib::protocol::Request,
    ) -> comenq_lib::protocol::Response {
        crate::listener::protocol::dispatch_request(queue, request).await
    }
}
