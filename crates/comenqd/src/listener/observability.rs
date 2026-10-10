//! Tests for bounded listener and blocking-store tracing fields.

use super::handle_client;
use crate::config::Config;
use crate::queue::SharedQueue;
use comenq_lib::protocol::Request;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

#[derive(Clone, Default)]
struct SpanCollector(Arc<Mutex<HashMap<u64, CapturedSpan>>>);

#[derive(Default)]
struct CapturedSpan {
    name: String,
    fields: BTreeMap<String, String>,
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

impl<S> Layer<S> for SpanCollector
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        let name = attrs.metadata().name();
        if !matches!(name, "handle_client" | "queue_store") {
            return;
        }
        let mut fields = FieldCollector::default();
        attrs.record(&mut fields);
        self.0.lock().expect("lock span collector").insert(
            id.clone().into_u64(),
            CapturedSpan {
                name: name.to_owned(),
                fields: fields.0,
            },
        );
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if ctx.span(id).is_none() {
            return;
        }
        let mut fields = FieldCollector::default();
        values.record(&mut fields);
        if let Some(captured) = self
            .0
            .lock()
            .expect("lock span collector")
            .get_mut(&id.clone().into_u64())
        {
            captured.fields.extend(fields.0);
        }
    }
}

#[test]
fn protocol_and_blocking_store_spans_record_bounded_fields() {
    let dir = tempfile::tempdir().expect("create queue directory");
    let config = Arc::new(Config::from(test_support::temp_config(&dir)));
    let queue = SharedQueue::open(config).expect("open queue");
    let collector = SpanCollector::default();
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(collector.clone()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build observability test runtime");

    tracing::dispatcher::with_default(&dispatch, || {
        runtime.block_on(async {
            let (mut client, server) = UnixStream::pair().expect("create socket pair");
            let request = serde_json::to_vec(&Request::List).expect("serialize list request");
            let (server_result, reply) =
                tokio::join!(handle_client(server, Arc::clone(&queue)), async move {
                    client.write_all(&request).await.expect("write request");
                    client.shutdown().await.expect("shutdown request");
                    let mut reply = Vec::new();
                    client.read_to_end(&mut reply).await.expect("read reply");
                    reply
                });
            server_result.expect("handle list request");
            let response: comenq_lib::protocol::Response =
                serde_json::from_slice(&reply).expect("decode list response");
            assert!(matches!(
                response,
                comenq_lib::protocol::Response::Ok { .. }
            ));
        });
    });

    let spans = collector.0.lock().expect("read captured spans");
    let listener = spans
        .values()
        .find(|span| span.name == "handle_client")
        .expect("listener span");
    assert_eq!(
        listener.fields.get("operation").map(String::as_str),
        Some("list")
    );
    assert_eq!(
        listener.fields.get("outcome").map(String::as_str),
        Some("accepted")
    );
    assert_eq!(
        listener.fields.get("error_kind").map(String::as_str),
        Some("none")
    );
    assert!(listener.fields.contains_key("elapsed_ms"));

    let store = spans
        .values()
        .find(|span| span.name == "queue_store")
        .expect("blocking store span");
    assert_eq!(
        store.fields.get("operation").map(String::as_str),
        Some("list")
    );
    assert_eq!(
        store.fields.get("outcome").map(String::as_str),
        Some("success")
    );
    assert_eq!(
        store.fields.get("error_kind").map(String::as_str),
        Some("none")
    );
    assert!(store.fields.contains_key("elapsed_ms"));
    assert!(store.fields.keys().all(|field| {
        matches!(
            field.as_str(),
            "operation" | "elapsed_ms" | "outcome" | "error_kind"
        )
    }));
}
