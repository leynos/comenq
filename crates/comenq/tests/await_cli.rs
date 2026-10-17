//! Exercise `await` through the production command-line binary.

use comenq_lib::protocol::{AwaitEvent, Request};
use std::error::Error;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::process::Command;

const COMMENT_URL: &str = "https://github.com/o/r/issues/1#issuecomment-1";
const AWAIT_ID: &str = "1a2b3c4d";

#[tokio::test]
async fn binary_await_sends_request_and_prints_the_comment_url() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let socket = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket)?;
    let child = Command::new(env!("CARGO_BIN_EXE_comenq"))
        .args(["--socket"])
        .arg(&socket)
        .args(["await", AWAIT_ID])
        .env("RUST_LOG", "off")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let output = tokio::time::timeout(Duration::from_secs(10), async {
        let (mut stream, _) = listener.accept().await?;
        let mut request_bytes = Vec::new();
        stream.read_to_end(&mut request_bytes).await?;
        let request: Request = serde_json::from_slice(&request_bytes)?;
        assert_eq!(
            request,
            Request::Await {
                id: AWAIT_ID.into()
            }
        );

        let mut frame = serde_json::to_vec(&AwaitEvent::Posted {
            comment_url: COMMENT_URL.into(),
        })?;
        frame.push(b'\n');
        stream.write_all(&frame).await?;

        Ok::<_, Box<dyn Error>>(child.wait_with_output().await?)
    })
    .await??;

    assert!(output.status.success());
    assert_eq!(output.stdout, format!("{COMMENT_URL}\n").as_bytes());
    assert!(output.stderr.is_empty());
    Ok(())
}
