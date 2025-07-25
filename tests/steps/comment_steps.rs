#![allow(clippy::expect_used, reason = "simplify test failure output")]

use comenq_lib::CommentRequest;
use cucumber::{World, given, then, when};

#[derive(Debug, Default, World)]
pub struct CommentWorld {
    request: Option<CommentRequest>,
    json: Option<String>,
    result: Option<Result<CommentRequest, serde_json::Error>>,
}

#[given("a default comment request")]
fn a_default_comment_request(world: &mut CommentWorld) {
    world.request = Some(CommentRequest {
        owner: "octocat".into(),
        repo: "hello-world".into(),
        pr_number: 1,
        body: "Hi".into(),
    });
}

#[when("it is serialised")]
fn it_is_serialised(world: &mut CommentWorld) {
    if let Some(req) = world.request.take() {
        world.json =
            Some(serde_json::to_string(&req).expect("serialisation should succeed in test"));
    }
}

#[then("the JSON is correct")]
fn the_json_is(world: &mut CommentWorld) {
    match world.json.take() {
        Some(actual) => {
            let act: serde_json::Value =
                serde_json::from_str(&actual).expect("test JSON should parse successfully");
            let exp = serde_json::json!({
                "owner": "octocat",
                "repo": "hello-world",
                "pr_number": 1,
                "body": "Hi"
            });
            assert_eq!(act, exp);
        }
        None => panic!("missing JSON output - test setup error"),
    }
}

#[given("invalid JSON")]
fn invalid_json(world: &mut CommentWorld) {
    world.json = Some("{ invalid json }".to_string());
}

#[given("valid JSON missing the 'owner' field")]
fn valid_json_missing_owner(world: &mut CommentWorld) {
    world.json = Some(
        r#"{
            "repo": "hello-world",
            "pr_number": 1,
            "body": "Hi"
        }"#
        .to_string(),
    );
}

#[given("valid JSON missing the 'repo' field")]
fn valid_json_missing_repo(world: &mut CommentWorld) {
    world.json = Some(
        r#"{
            "owner": "octocat",
            "pr_number": 1,
            "body": "Hi"
        }"#
        .to_string(),
    );
}

#[given("valid JSON missing all required fields")]
fn valid_json_missing_all_required_fields(world: &mut CommentWorld) {
    world.json = Some(r#"{"body": "Hi"}"#.to_string());
}

#[when("it is parsed")]
fn it_is_parsed(world: &mut CommentWorld) {
    if let Some(json) = world.json.clone() {
        world.result = Some(serde_json::from_str(&json));
    }
}

#[then("an error is returned")]
fn an_error_is_returned(world: &mut CommentWorld) {
    match world.result.take() {
        Some(res) => assert!(res.is_err()),
        None => panic!("no parse result"),
    }
}
