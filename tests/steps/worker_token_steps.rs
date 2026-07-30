//! Behavioural test steps for round-robin token rotation.
//!
//! Extends the worker scenarios with a two-token pool, asserting that
//! posts alternate tokens and that rotation resumes from the hash the
//! latest history record carries.

use anyhow::Context as _;
use comenq_lib::CommentRequest;
use comenqd::daemon::token_hash;
use comenqd::store::{HistoryRecord, QueueStore, StoredEntry};
use cucumber::{given, then};

use super::worker_steps::{WorkerWorld, listed_history};

/// Rotation pool used by the token scenarios, in configured order.
pub(crate) const ROTATION_TOKENS: [(&str, &str); 2] = [
    ("pandalump-token", "alpha-token"),
    ("buzzybee-token", "bravo-token"),
];

#[given("two GitHub tokens are configured")]
fn two_tokens_configured(world: &mut WorkerWorld) {
    world.tokens = ROTATION_TOKENS
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
}

#[given("the history already records a post with the second token")]
fn history_seeded_with_second_token(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let queue = world.queue.as_ref().context("queue initialized")?;
    let store = QueueStore::open(&queue.config().queue_path).context("open store")?;
    let (_, second_value) = ROTATION_TOKENS.get(1).context("second rotation token")?;
    let posted = StoredEntry {
        id: "deadbeef".into(),
        order: 0,
        flutter_seconds: 0,
        enqueued_at: 0,
        not_before: 0,
        request: CommentRequest {
            owner: "o".into(),
            repo: "r".into(),
            pr_number: 1,
            body: "earlier".into(),
        },
    };
    store
        .append_history(&HistoryRecord::success(
            &posted,
            &token_hash(second_value),
            1,
        ))
        .context("seed history")?;
    Ok(())
}

#[then("the posting history alternates between the two tokens")]
async fn history_alternates(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let queue = world.queue.as_ref().context("queue initialized")?.clone();
    let history = listed_history(&queue).await?;
    let hashes: Vec<&str> = history
        .iter()
        .map(|record| record.token_hash.as_str())
        .collect();
    let expected: Vec<String> = ROTATION_TOKENS
        .iter()
        .map(|(_, value)| token_hash(value))
        .collect();
    assert_eq!(
        hashes,
        expected.iter().map(String::as_str).collect::<Vec<_>>(),
        "the first post should use the first token, the second the next"
    );
    let server = world
        .server
        .as_ref()
        .context("server should be initialized")?;
    let auth_headers: Vec<String> = server
        .received_requests()
        .await
        .context("inbound requests should be recorded")?
        .iter()
        .map(|request| {
            request
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert_eq!(auth_headers.len(), 2, "both posts should reach GitHub");
    assert_ne!(
        auth_headers.first(),
        auth_headers.get(1),
        "each post should authenticate with a different token"
    );
    world.shutdown_and_join().await;
    Ok(())
}

#[then("the newest history record uses the first token")]
async fn newest_record_uses_first_token(world: &mut WorkerWorld) -> anyhow::Result<()> {
    let queue = world.queue.as_ref().context("queue initialized")?.clone();
    let history = listed_history(&queue).await?;
    let newest = history.last().context("history should hold records")?;
    let (_, first_value) = ROTATION_TOKENS.first().context("first rotation token")?;
    assert_eq!(
        newest.token_hash,
        token_hash(first_value),
        "rotation should wrap from the second token back to the first"
    );
    Ok(())
}
