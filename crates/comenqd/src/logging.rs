//! Logging utilities for the daemon.
//!
//! Initializes structured logging using `tracing` and
//! `tracing-subscriber`, reading filter settings from the `RUST_LOG`
//! environment variable.

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::{EnvFilter, fmt};

/// Initialize the global tracing subscriber.
///
/// Call `init` before any logging statements to avoid missing logs.
///
/// # Examples
///
/// ```rust,no_run
/// use comenqd::logging::init;
///
/// // Initialize logging as early as possible.
/// init();
/// tracing::info!("Logging is initialized!");
/// ```
pub fn init() {
    init_with_writer(fmt::writer::BoxMakeWriter::new(std::io::stdout));
}

/// Initialize logging with a custom writer.
///
/// # Examples
///
/// ```rust,no_run
/// use comenqd::logging::init_with_writer;
/// use tracing_subscriber::fmt;
///
/// init_with_writer(fmt::writer::BoxMakeWriter::new(std::io::stdout));
/// ```
pub fn init_with_writer<W>(writer: W)
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(writer)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing::info;

    #[derive(Clone)]
    struct BufMakeWriter {
        buf: Arc<Mutex<Vec<u8>>>,
    }

    impl<'a> MakeWriter<'a> for BufMakeWriter {
        type Writer = BufWriter;

        fn make_writer(&'a self) -> Self::Writer {
            BufWriter {
                buf: self.buf.clone(),
            }
        }
    }

    struct BufWriter {
        buf: Arc<Mutex<Vec<u8>>>,
    }

    impl std::io::Write for BufWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.buf
                .lock()
                .expect("Failed to lock log buffer")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn init_logging() {
        let buf = Arc::new(Mutex::new(Vec::new()));
        unsafe { std::env::set_var("RUST_LOG", "info") };
        init_with_writer(BufMakeWriter { buf: buf.clone() });
        info!("captured");
        let output = String::from_utf8(buf.lock().expect("Failed to lock log buffer").clone())
            .expect("Captured output is not valid UTF-8");
        assert!(output.contains("captured"));
    }
}
