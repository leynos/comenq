//! Logging utilities for the daemon.
//!
//! Initializes structured logging using `tracing` and
//! `tracing-subscriber`, reading filter settings from the `RUST_LOG`
//! environment variable.

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::{EnvFilter, fmt};

/// Initialize the global tracing subscriber.
pub fn init() {
    init_with_writer(fmt::writer::BoxMakeWriter::new(std::io::stdout));
}

/// Initialize logging with a custom writer.
pub fn init_with_writer<W>(writer: W)
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(writer)
        .json()
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
            self.buf.lock().expect("lock buffer").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn init_logging() {
        let buf = Arc::new(Mutex::new(Vec::new()));
        std::env::set_var("RUST_LOG", "info");
        init_with_writer(BufMakeWriter { buf: buf.clone() });
        info!("captured");
        let output = String::from_utf8(buf.lock().expect("lock buffer").clone()).expect("utf8");
        assert!(output.contains("captured"));
    }
}
