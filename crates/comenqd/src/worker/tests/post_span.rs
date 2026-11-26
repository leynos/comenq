//! Structured-tracing tests for bounded GitHub post outcomes.

use super::super::posting::post_comment_with_metrics;
use crate::config::Config;
use comenq_lib::CommentRequest;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;
use test_support::octocrab_for;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const POST_SPAN: &str = "post_comment_with_metrics";

fn config_with_api_timeout(github_api_timeout_secs: u64) -> (tempfile::TempDir, Config) {
    let dir = tempdir().expect("create temporary configuration directory");
    let mut config: Config = test_support::temp_config(&dir).into();
    config.github_api_timeout_secs = github_api_timeout_secs;
    (dir, config)
}

#[derive(Clone, Default)]
struct PostSpanCollector {
    spans: Arc<Mutex<HashMap<u64, CapturedSpan>>>,
}

#[derive(Debug, Default)]
struct CapturedSpan {
    initial_fields: BTreeMap<String, String>,
    recorded_fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldCollector(BTreeMap<String, String>);

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

impl<S> Layer<S> for PostSpanCollector
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        if attrs.metadata().name() != POST_SPAN {
            return;
        }
        let mut fields = FieldCollector::default();
        attrs.record(&mut fields);
        self.spans.lock().expect("lock post-span collector").insert(
            id.clone().into_u64(),
            CapturedSpan {
                initial_fields: fields.0,
                recorded_fields: BTreeMap::new(),
            },
        );
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut fields = FieldCollector::default();
        values.record(&mut fields);
        if let Some(span) = self
            .spans
            .lock()
            .expect("lock post-span collector")
            .get_mut(&id.clone().into_u64())
        {
            span.recorded_fields.extend(fields.0);
        }
    }
}

/// Mount a POST handler for the comment endpoint that answers with `response`.
async fn comment_server(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/owner/repo/issues/1/comments"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

/// Post `request` to a server answering with `response`, under the given API
/// timeout, returning the comment URL when the post succeeds.
async fn post_against(
    response: ResponseTemplate,
    request: &CommentRequest,
    api_timeout_secs: u64,
) -> Option<String> {
    let server = comment_server(response).await;
    let client = octocrab_for(&server).expect("build client");
    let (_dir, config) = config_with_api_timeout(api_timeout_secs);
    post_comment_with_metrics(&client, request, &config)
        .await
        .ok()
}

/// Post once each to a succeeding, an erroring and a timing-out server.
async fn post_every_outcome(request: &CommentRequest) {
    let response_body: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/github_comment_response.json"
    ))
    .expect("parse GitHub comment response fixture");
    let success = ResponseTemplate::new(201).set_body_json(response_body);
    assert_eq!(
        post_against(success, request, 1).await.as_deref(),
        Some("https://github.com/o/r/issues/1#issuecomment-1"),
        "the post succeeds"
    );
    assert!(
        post_against(ResponseTemplate::new(500), request, 1)
            .await
            .is_none(),
        "a server error fails the post"
    );
    let slow = ResponseTemplate::new(201).set_delay(Duration::from_secs(1));
    assert!(
        post_against(slow, request, 0).await.is_none(),
        "a slow server times the post out"
    );
}

#[test]
fn github_post_span_records_only_bounded_outcomes() {
    let request = CommentRequest {
        owner: "owner".into(),
        repo: "repo".into(),
        pr_number: 1,
        body: "body".into(),
    };
    let collector = PostSpanCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let dispatch = tracing::Dispatch::new(subscriber);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build post-span test runtime");

    tracing::dispatcher::with_default(&dispatch, || {
        runtime.block_on(post_every_outcome(&request));
    });

    let spans = collector.spans.lock().expect("read post-span collector");
    assert_eq!(spans.len(), 3);
    let outcomes = spans
        .values()
        .map(|span| {
            assert_eq!(span.initial_fields.get("task"), Some(&"worker".to_owned()));
            assert!(
                span.initial_fields
                    .keys()
                    .all(|field| matches!(field.as_str(), "task" | "outcome"))
            );
            assert_eq!(
                span.recorded_fields.keys().collect::<Vec<_>>(),
                vec!["outcome"]
            );
            span.recorded_fields
                .get("outcome")
                .expect("recorded post outcome")
                .clone()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        outcomes,
        BTreeSet::from([
            "api_error".to_owned(),
            "success".to_owned(),
            "timeout".to_owned(),
        ])
    );
}
