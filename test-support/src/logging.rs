//! Logging utilities for tests.
//!
//! Provides test-safe logging initialisation that avoids reading from the
//! environment.

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::{EnvFilter, fmt};

/// Initialise logging with a custom writer and explicit filter.
///
/// This avoids reading from the environment, making it suitable for tests
/// where environment mutation is forbidden.
///
/// # Examples
///
/// ```rust,no_run
/// use test_support::logging::init_with_writer_and_filter;
/// use tracing_subscriber::fmt;
///
/// init_with_writer_and_filter(fmt::writer::BoxMakeWriter::new(std::io::stdout), "info");
/// ```
pub fn init_with_writer_and_filter<W>(writer: W, filter: &str)
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_writer(writer)
        .init();
}
