//! Exercise `await` through the production command-line binary.

use comenq_lib::protocol::{AwaitEvent, Request, Response};
use rstest::{fixture, rstest};
use std::error::Error;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::process::{Child, Command};

const COMMENT_URL: &str = "https://github.com/o/r/issues/1#issuecomment-1";
const AWAIT_ID: &str = "1a2b3c4d";

/// A bound fixture socket and the awaiting client connected to it.
struct AwaitFixture {
    /// Kept alive so the socket path outlives the test.
    _directory: tempfile::TempDir,
    listener: UnixListener,
    child: Child,
}

/// Spawn the real binary in `await` mode against a fixture socket.
fn spawn_awaiter(socket: &Path) -> Result<Child, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_comenq"))
        .args(["--socket"])
        .arg(socket)
        .args(["await", AWAIT_ID])
        .env("RUST_LOG", "off")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?)
}

/// Bind a fixture socket and start one awaiting client against it.
#[fixture]
async fn await_fixture() -> AwaitFixture {
    let directory = tempfile::tempdir().expect("create fixture directory");
    let socket = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).expect("bind fixture socket");
    let child = spawn_awaiter(&socket).expect("spawn await client");
    AwaitFixture {
        _directory: directory,
        listener,
        child,
    }
}

#[rstest]
#[tokio::test]
async fn binary_await_sends_request_and_prints_the_comment_url(
    #[future]
    #[from(await_fixture)]
    fixture: AwaitFixture,
) -> Result<(), Box<dyn Error>> {
    let AwaitFixture {
        _directory,
        listener,
        child,
    } = fixture.await;

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

#[rstest]
#[tokio::test]
async fn binary_reports_a_legacy_daemon_rejection(
    #[future]
    #[from(await_fixture)]
    fixture: AwaitFixture,
) -> Result<(), Box<dyn Error>> {
    const REJECTION: &str = concat!(
        "invalid request: unknown variant `await`, expected one of ",
        "`put`, `list`, `bump`, `bust`, `del`, `hist` at line 1 column 1"
    );
    let AwaitFixture {
        _directory,
        listener,
        child,
    } = fixture.await;

    let output = tokio::time::timeout(Duration::from_secs(10), async {
        let (mut stream, _) = listener.accept().await?;
        let mut request_bytes = Vec::new();
        stream.read_to_end(&mut request_bytes).await?;
        // A daemon that predates await answers with a one-shot error and no
        // trailing newline, then closes the connection.
        stream
            .write_all(&serde_json::to_vec(&Response::error(REJECTION))?)
            .await?;
        stream.shutdown().await?;
        Ok::<_, Box<dyn Error>>(child.wait_with_output().await?)
    })
    .await??;

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr)?,
        concat!(
            "daemon rejected the await request: ",
            "invalid request: unknown variant `await`, expected one of ",
            "`put`, `list`, `bump`, `bust`, `del`, `hist` at line 1 column 1",
            " (comenqd may predate await; run comenq and comenqd from the same release)\n",
        )
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn binary_reports_an_unknown_identifier_from_the_stream(
    #[future]
    #[from(await_fixture)]
    fixture: AwaitFixture,
) -> Result<(), Box<dyn Error>> {
    let AwaitFixture {
        _directory,
        listener,
        child,
    } = fixture.await;

    let output = tokio::time::timeout(Duration::from_secs(10), async {
        let (mut stream, _) = listener.accept().await?;
        let mut request_bytes = Vec::new();
        stream.read_to_end(&mut request_bytes).await?;
        let mut frame = serde_json::to_vec(&AwaitEvent::Error {
            message: "unknown comment identifier".into(),
        })?;
        frame.push(b'\n');
        stream.write_all(&frame).await?;
        stream.shutdown().await?;
        Ok::<_, Box<dyn Error>>(child.wait_with_output().await?)
    })
    .await??;

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "daemon refused the request: unknown comment identifier\n"
    );
    Ok(())
}
