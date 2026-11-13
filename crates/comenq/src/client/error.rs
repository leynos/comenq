//! Client-side error type shared by the transport and command layers.

use thiserror::Error;

/// Errors that can occur when interacting with the daemon.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Connecting to the daemon failed.
    #[error("failed to connect to daemon: {0}")]
    Connect(#[from] std::io::Error),
    /// Serializing the request or parsing the reply failed.
    #[error("failed to encode or decode a daemon message: {0}")]
    Serialize(#[from] serde_json::Error),
    /// The serialized request exceeds the daemon's request-size limit.
    #[error("request payload is {size} bytes, exceeding the {limit}-byte limit")]
    RequestTooLarge {
        /// Serialized request size in bytes.
        size: usize,
        /// Maximum request size accepted by the daemon, in bytes.
        limit: usize,
    },
    /// Writing the request to the socket failed.
    #[error("failed to write to daemon: {0}")]
    Write(#[source] std::io::Error),
    /// Shutting down the socket failed.
    #[error("failed to close connection: {0}")]
    Shutdown(#[source] std::io::Error),
    /// Reading the daemon's reply failed.
    #[error("failed to read daemon reply: {0}")]
    Read(#[source] std::io::Error),
    /// The daemon reply exceeds the protocol size limit.
    #[error("daemon reply exceeds the protocol size limit")]
    ReplyTooLarge,
    /// The streaming connection ended after a frame but before a terminal one.
    ///
    /// Only reported once the daemon has sent at least one await frame, so a
    /// lost posting connection is never confused with a daemon that never
    /// spoke the streaming protocol.
    #[error("connection lost before posting completed")]
    ConnectionLost,
    /// The daemon rejected the await request before streaming any event.
    ///
    /// An older daemon answers an unknown request variant with a one-shot
    /// [`Response::Error`](comenq_lib::protocol::Response::Error), which
    /// arrives in place of the first await frame.
    #[error(
        "daemon rejected the await request: {0} (comenqd may predate await; run comenq and comenqd from the same release)"
    )]
    AwaitRejected(String),
    /// The daemon closed the await connection before sending any reply.
    ///
    /// An empty or unparseable first reply most often means the daemon predates
    /// the streaming protocol and closed the connection instead of answering.
    #[error(
        "daemon closed the connection without a reply (possible protocol mismatch with an older daemon; run comenq and comenqd from the same release)"
    )]
    NoInitialReply,
    /// The daemon did not finish its reply before the client deadline.
    #[error("timed out waiting for daemon reply")]
    ReplyTimeout,
    /// The daemon reported a failure.
    #[error("daemon refused the request: {0}")]
    Daemon(String),
    /// The daemon's reply did not match the request.
    #[error("unexpected reply from daemon")]
    UnexpectedResponse,
    /// Writing the command result to standard output failed.
    #[error("failed to write command output: {0}")]
    Output(#[source] std::io::Error),
}
