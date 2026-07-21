//! Token hashing and rotation selection tests.

use super::super::{TokenClient, build_octocrab, next_token_index, token_hash};
use std::sync::Arc;

fn clients() -> Vec<TokenClient> {
    ["alpha-token", "bravo-token"]
        .iter()
        .map(|token| {
            let octocrab = Arc::new(build_octocrab(token).expect("build octocrab"));
            TokenClient::new(format!("{token}-file"), token, octocrab)
        })
        .collect()
}

#[test]
fn token_hash_is_sha256_hex() {
    assert_eq!(
        token_hash("s3cret"),
        "1ec1c26b50d5d3c58d9583181af8076655fe00756bf7285940ba3670f99fcba0"
    );
}

#[test]
fn empty_or_unrecognized_history_starts_at_the_first_token() {
    let clients = clients();
    assert_eq!(next_token_index(&clients, None), 0);
    assert_eq!(next_token_index(&clients, Some("not-a-hash")), 0);
}

#[test]
fn rotation_advances_and_wraps_after_the_last_matching_token() {
    let clients = clients();
    let first_hash = token_hash("alpha-token");
    let second_hash = token_hash("bravo-token");
    assert_eq!(next_token_index(&clients, Some(&first_hash)), 1);
    assert_eq!(next_token_index(&clients, Some(&second_hash)), 0);
}
