//! Adapt JSON protocol requests and queue results at the listener boundary.

use comenq_lib::protocol::{MAX_PENDING_ENTRIES, MAX_RESPONSE_BYTES, Request, Response};

use crate::queue::SharedQueue;
use crate::store::{Result as StoreResult, StoreError, StoredEntry};

/// Adapt one protocol request into a queue operation and its wire response.
pub(crate) async fn dispatch_request(queue: &SharedQueue, request: Request) -> Response {
    dispatch_request_with_error_kind(queue, request).await.0
}

pub(crate) async fn dispatch_request_with_error_kind(
    queue: &SharedQueue,
    request: Request,
) -> (Response, Option<&'static str>) {
    match request {
        Request::Put { request, immediate } => match queue.put(request, immediate).await {
            Ok((entry, eta)) => (Response::entry(entry.to_pending(eta)), None),
            Err(error) => store_error_response(error),
        },
        Request::List => match queue.list().await {
            Ok(schedule) => match response_for_schedule(schedule) {
                Ok(response) => {
                    let kind =
                        matches!(response, Response::Error { .. }).then_some("response_too_large");
                    (response, kind)
                }
                Err(_) => (
                    Response::error("failed to encode list response"),
                    Some("response_serialization"),
                ),
            },
            Err(error) => store_error_response(error),
        },
        Request::Bump { id } => map_store_response(queue.bump(&id).await),
        Request::Bust { id } => map_store_response(queue.bust(&id).await),
        Request::Del { id } => map_store_response(queue.del(&id).await),
    }
}

fn map_store_response(result: StoreResult<()>) -> (Response, Option<&'static str>) {
    match result {
        Ok(()) => (Response::ok(), None),
        Err(error) => store_error_response(error),
    }
}

fn store_error_response(error: StoreError) -> (Response, Option<&'static str>) {
    let category = error.category();
    (Response::error(error.to_string()), Some(category))
}

/// Build the ordered client response unless its serialized form exceeds the protocol limit.
///
/// Returning an error keeps the daemon from sending a response that the client
/// would reject for exceeding the shared wire-size limit.
fn response_for_schedule(
    schedule: Vec<(StoredEntry, u64)>,
) -> std::result::Result<Response, serde_json::Error> {
    let mut projected_size = serde_json::to_vec(&Response::entries(Vec::new()))?.len();
    let mut entries = Vec::with_capacity(schedule.len().min(MAX_PENDING_ENTRIES));

    for (entry, eta) in schedule.into_iter().take(MAX_PENDING_ENTRIES) {
        let pending = entry.to_pending(eta);
        let pending_size = serde_json::to_vec(&pending)?.len();
        let separator_size = usize::from(!entries.is_empty());
        let next_size = projected_size
            .saturating_add(pending_size)
            .saturating_add(separator_size);
        if next_size > MAX_RESPONSE_BYTES {
            return Ok(Response::error(format!(
                "list response exceeds the {MAX_RESPONSE_BYTES}-byte limit"
            )));
        }
        projected_size = next_size;
        entries.push(pending);
    }

    Ok(Response::entries(entries))
}
