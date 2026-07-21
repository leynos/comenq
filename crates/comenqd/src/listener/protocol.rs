//! Adapt JSON protocol requests and queue results at the listener boundary.

use comenq_lib::protocol::{
    HistoryCursor, HistoryEntry, MAX_PENDING_ENTRIES, MAX_RESPONSE_BYTES, PendingEntry, Request,
    Response,
};

use crate::queue::SharedQueue;
use crate::store::{
    HistoryPage, HistoryPosition, HistoryRecord, Result as StoreResult, StoreError, StoredEntry,
};

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
            Ok((entry, eta)) => (Response::entry(pending_entry(entry, eta)), None),
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
        Request::Hist { limit, cursor } => {
            let position = cursor.map(|cursor| HistoryPosition {
                offset: cursor.offset,
                end_offset: cursor.end_offset,
            });
            match queue.history_page(limit, position).await {
                Ok(page) => match response_for_history(page) {
                    Ok(response) => {
                        let kind = matches!(response, Response::Error { .. })
                            .then_some("response_too_large");
                        (response, kind)
                    }
                    Err(_) => (
                        Response::error("failed to encode history response"),
                        Some("response_serialization"),
                    ),
                },
                Err(error) => store_error_response(error),
            }
        }
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
        let pending = pending_entry(entry, eta);
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

/// Build a chronologically ordered history page within the shared wire limit.
fn response_for_history(page: HistoryPage) -> std::result::Result<Response, serde_json::Error> {
    let record_count = page.records.len();
    let mut entries = Vec::with_capacity(record_count);
    let mut encoded_entries_size = 0;
    let mut next_cursor = None;

    for (index, item) in page.records.into_iter().enumerate() {
        let entry = history_entry(item.record);
        let more_in_page = index + 1 < record_count;
        let has_more = more_in_page || page.next.is_some();
        let candidate_cursor = has_more.then(|| {
            let position = item.position_after;
            HistoryCursor {
                offset: position.offset,
                end_offset: position.end_offset,
            }
        });
        let entry_size = serde_json::to_vec(&entry)?.len();
        if history_response_size(
            encoded_entries_size + entry_size + usize::from(!entries.is_empty()),
            candidate_cursor,
        )? > MAX_RESPONSE_BYTES
        {
            if entries.is_empty() {
                return Ok(Response::error(format!(
                    "one history record exceeds the {MAX_RESPONSE_BYTES}-byte response limit"
                )));
            }
            break;
        }
        encoded_entries_size += entry_size + usize::from(!entries.is_empty());
        entries.push(entry);
        next_cursor = candidate_cursor;
    }

    if let Some(position) = page.next
        && entries.is_empty()
    {
        next_cursor = Some(HistoryCursor {
            offset: position.offset,
            end_offset: position.end_offset,
        });
    }
    Ok(Response::history(entries, next_cursor))
}

fn history_response_size(
    encoded_entries_size: usize,
    next_cursor: Option<HistoryCursor>,
) -> std::result::Result<usize, serde_json::Error> {
    let empty = serde_json::to_vec(&Response::history(Vec::new(), next_cursor))?;
    Ok(empty.len() + encoded_entries_size)
}

fn history_entry(record: HistoryRecord) -> HistoryEntry {
    HistoryEntry {
        id: record.id,
        posted_at: record.posted_at,
        success: record.success,
        error: record.error,
        token_hash: record.token_hash,
        owner: record.request.owner,
        repo: record.request.repo,
        pr_number: record.request.pr_number,
        body: record.request.body,
    }
}

/// Adapt stored data to the wire shape only at the transport boundary.
pub(crate) fn pending_entry(entry: StoredEntry, eta_seconds: u64) -> PendingEntry {
    PendingEntry {
        id: entry.id,
        eta_seconds,
        owner: entry.request.owner,
        repo: entry.request.repo,
        pr_number: entry.request.pr_number,
        body: entry.request.body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{HistoryPosition, HistoryRecordAt};
    use comenq_lib::CommentRequest;

    fn history_record(body: String) -> HistoryRecord {
        HistoryRecord {
            attempt_id: "attempt".into(),
            id: "1a2b3c4d".into(),
            posted_at: 1_700_000_000,
            success: false,
            error: Some("request timed out".into()),
            token_hash: "ab".repeat(32),
            request: CommentRequest {
                owner: "octocat".into(),
                repo: "hello-world".into(),
                pr_number: 7,
                body,
            },
        }
    }

    #[test]
    fn history_response_maps_store_records_at_the_listener_boundary() {
        let page = HistoryPage {
            records: vec![HistoryRecordAt {
                record: history_record("body".into()),
                position_after: HistoryPosition {
                    offset: 100,
                    end_offset: 100,
                },
            }],
            next: None,
        };

        let Response::Ok {
            history: Some(entries),
            next_cursor: None,
            ..
        } = response_for_history(page).expect("encode history response")
        else {
            panic!("expected history response");
        };

        assert_eq!(entries[0].owner, "octocat");
        assert_eq!(entries[0].repo, "hello-world");
        assert_eq!(entries[0].pr_number, 7);
        assert_eq!(entries[0].body, "body");
        assert_eq!(entries[0].error.as_deref(), Some("request timed out"));
    }

    #[test]
    fn history_response_stops_at_the_wire_size_limit_and_returns_a_cursor() {
        let page = HistoryPage {
            records: (0..2)
                .map(|index| HistoryRecordAt {
                    record: history_record("x".repeat(1_200_000)),
                    position_after: HistoryPosition {
                        offset: (index + 1) * 1_200_000,
                        end_offset: 2_400_000,
                    },
                })
                .collect(),
            next: None,
        };

        let response = response_for_history(page).expect("encode bounded history page");
        let encoded = serde_json::to_vec(&response).expect("serialize history response");

        assert!(encoded.len() <= MAX_RESPONSE_BYTES);
        let Response::Ok {
            history: Some(entries),
            next_cursor: Some(cursor),
            ..
        } = response
        else {
            panic!("expected a truncated history page with a cursor");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(cursor.offset, 1_200_000);
        assert_eq!(cursor.end_offset, 2_400_000);
    }
}
