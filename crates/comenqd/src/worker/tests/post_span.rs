//! Structured-tracing tests for bounded GitHub post outcomes.

use super::super::post_comment_with_metrics;
use super::config_with_flutter;
use comenq_lib::CommentRequest;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use test_support::octocrab_for;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::instrument::WithSubscriber;
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const POST_SPAN: &str = "post_comment_with_metrics";

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
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if ctx
            .span(id)
            .is_none_or(|span| span.metadata().name() != POST_SPAN)
        {
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

#[tokio::test(flavor = "current_thread")]
async fn github_post_span_records_only_bounded_outcomes() {
    let request = CommentRequest {
        owner: "owner".into(),
        repo: "repo".into(),
        pr_number: 1,
        body: "body".into(),
    };
    let collector = PostSpanCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());

    async {
        let success_server = MockServer::start().await;
        let response_body: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/github_comment_response.json"
        ))
        .expect("parse GitHub comment response fixture");
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues/1/comments"))
            .respond_with(ResponseTemplate::new(201).set_body_json(response_body))
            .mount(&success_server)
            .await;
        let success_client = octocrab_for(&success_server).expect("build success client");
        assert!(
            post_comment_with_metrics(&success_client, &request, &config_with_flutter(1, 0))
                .await
                .is_ok()
        );

        let error_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues/1/comments"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&error_server)
            .await;
        let error_client = octocrab_for(&error_server).expect("build error client");
        assert!(
            post_comment_with_metrics(&error_client, &request, &config_with_flutter(1, 0))
                .await
                .is_err()
        );

        let timeout_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues/1/comments"))
            .respond_with(ResponseTemplate::new(201).set_delay(Duration::from_secs(1)))
            .mount(&timeout_server)
            .await;
        let timeout_client = octocrab_for(&timeout_server).expect("build timeout client");
        let mut timeout_config = config_with_flutter(1, 0);
        timeout_config.github_api_timeout_secs = 0;
        assert!(
            post_comment_with_metrics(&timeout_client, &request, &timeout_config)
                .await
                .is_err()
        );
    }
    .with_subscriber(subscriber)
    .await;

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
