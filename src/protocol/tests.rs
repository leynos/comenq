//! Serialization round-trip tests for the client-daemon protocol.
use super::{HistoryCursor, HistoryEntry, PendingEntry, Request, Response};
use crate::CommentRequest;

fn sample_entry() -> PendingEntry {
    PendingEntry {
        id: "0011aabb".into(),
        eta_seconds: 120,
        owner: "octocat".into(),
        repo: "hello-world".into(),
        pr_number: 7,
        body: "Hi".into(),
    }
}

fn sample_history() -> HistoryEntry {
    HistoryEntry {
        id: "0011aabb".into(),
        posted_at: 1_700_000_000,
        success: false,
        error: Some("timeout".into()),
        token_hash: "ab".repeat(32),
        owner: "octocat".into(),
        repo: "hello-world".into(),
        pr_number: 7,
        body: "Hi".into(),
    }
}

#[test]
fn put_round_trips_through_json() {
    let req = Request::Put {
        request: CommentRequest {
            owner: "octocat".into(),
            repo: "hello-world".into(),
            pr_number: 7,
            body: "Hi".into(),
        },
        immediate: false,
    };
    let json = serde_json::to_string(&req).unwrap_or_else(|e| panic!("serialize: {e}"));
    assert!(json.contains(r#""op":"put""#), "missing op tag: {json}");
    let back: Request = serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
    assert_eq!(back, req);
}

#[test]
fn id_operations_round_trip_through_json() {
    for req in [
        Request::List,
        Request::Bump {
            id: "0011aabb".into(),
        },
        Request::Bust {
            id: "0011aabb".into(),
        },
        Request::Del {
            id: "0011aabb".into(),
        },
        Request::Hist {
            limit: None,
            cursor: None,
        },
        Request::Hist {
            limit: Some(20),
            cursor: None,
        },
        Request::Hist {
            limit: None,
            cursor: Some(HistoryCursor {
                offset: 10,
                end_offset: 20,
            }),
        },
    ] {
        let json = serde_json::to_string(&req).unwrap_or_else(|e| panic!("serialize: {e}"));
        let back: Request =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
        assert_eq!(back, req);
    }
}

#[test]
fn responses_round_trip_through_json() {
    for resp in [
        Response::ok(),
        Response::entry(sample_entry()),
        Response::entries(vec![sample_entry()]),
        Response::history(vec![sample_history()], None),
        Response::history(
            vec![sample_history()],
            Some(HistoryCursor {
                offset: 10,
                end_offset: 20,
            }),
        ),
        Response::error("nope"),
    ] {
        let json = serde_json::to_string(&resp).unwrap_or_else(|e| panic!("serialize: {e}"));
        let back: Response =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
        assert_eq!(back, resp);
    }
}

#[test]
fn history_entries_without_token_hash_remain_readable() -> Result<(), String> {
    let mut value = serde_json::to_value(sample_history()).map_err(|error| error.to_string())?;
    value
        .as_object_mut()
        .ok_or_else(|| "serialized history must be an object".to_owned())?
        .remove("token_hash")
        .ok_or_else(|| "sample history must contain a token hash".to_owned())?;

    let entry: HistoryEntry = serde_json::from_value(value).map_err(|error| error.to_string())?;

    assert_eq!(entry.token_hash, "");
    Ok(())
}

#[test]
fn put_defaults_to_deferred_posting() {
    let json = concat!(
        r#"{"op":"put","request":{"owner":"o","repo":"r","#,
        r#""pr_number":1,"body":"b"}}"#
    );
    let req: Request = serde_json::from_str(json).unwrap_or_else(|e| panic!("parse: {e}"));
    assert!(matches!(
        req,
        Request::Put {
            immediate: false,
            ..
        }
    ));
}

#[test]
fn hist_defaults_to_no_limit() {
    let req: Request =
        serde_json::from_str(r#"{"op":"hist"}"#).unwrap_or_else(|e| panic!("parse: {e}"));
    assert_eq!(
        req,
        Request::Hist {
            limit: None,
            cursor: None,
        }
    );
}

#[test]
fn unknown_operation_fails_to_parse() {
    let result: Result<Request, _> = serde_json::from_str(r#"{"op":"zap"}"#);
    assert!(result.is_err());
}

#[rstest::rstest]
#[case::waiting(crate::protocol::AwaitEvent::Waiting { eta_seconds: 60 }, "waiting")]
#[case::posted(crate::protocol::AwaitEvent::Posted { comment_url: "https://github.com/o/r/issues/1#issuecomment-1".into() }, "posted")]
#[case::error(crate::protocol::AwaitEvent::Error { message: "removed".into() }, "error")]
fn await_events_round_trip(
    #[case] event: crate::protocol::AwaitEvent,
    #[case] status: &str,
) -> Result<(), serde_json::Error> {
    let bytes = serde_json::to_vec(&event)?;
    let decoded: crate::protocol::AwaitEvent = serde_json::from_slice(&bytes)?;
    assert_eq!(decoded, event);
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        value.get("status").and_then(serde_json::Value::as_str),
        Some(status)
    );
    Ok(())
}

#[test]
fn await_request_round_trip() -> Result<(), serde_json::Error> {
    let request = Request::Await {
        id: "1a2b3c4d".into(),
    };
    let bytes = serde_json::to_vec(&request)?;
    assert_eq!(bytes, br#"{"op":"await","id":"1a2b3c4d"}"#);
    let decoded: Request = serde_json::from_slice(&bytes)?;
    assert_eq!(decoded, request);
    Ok(())
}
