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
        world.json = match serde_json::to_string(&req) {
            Ok(json) => Some(json),
            Err(e) => panic!("serialisation failed: {e}"),
        };
    }
}

#[then("the JSON is correct")]
fn the_json_is(world: &mut CommentWorld) {
    match world.json.take() {
        Some(actual) => {
            let act: serde_json::Value = serde_json::from_str(&actual)
                .unwrap_or_else(|e| panic!("parse actual failed: {e}"));
            let exp = serde_json::json!({
                "owner": "octocat",
                "repo": "hello-world",
                "pr_number": 1,
                "body": "Hi"
            });
            assert_eq!(act, exp);
        }
        None => panic!("missing json output"),
    }
}

#[given("invalid JSON")]
fn invalid_json(world: &mut CommentWorld) {
    world.json = Some("{ invalid json }".to_string());
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
