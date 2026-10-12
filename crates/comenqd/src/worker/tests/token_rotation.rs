//! Token hashing and rotation selection tests.

use super::super::{next_token_index, token_hash};

#[test]
fn token_hash_is_sha256_hex() {
    assert_eq!(
        token_hash("s3cret"),
        "1ec1c26b50d5d3c58d9583181af8076655fe00756bf7285940ba3670f99fcba0"
    );
}

#[test]
fn empty_or_unrecognized_history_starts_at_the_first_token() {
    let hashes = ["first-hash", "second-hash"];
    assert_eq!(next_token_index(hashes.iter().copied(), None), 0);
    assert_eq!(
        next_token_index(hashes.iter().copied(), Some("not-a-hash")),
        0
    );
}

#[test]
fn rotation_advances_and_wraps_after_the_last_matching_token() {
    let hashes = ["first-hash", "second-hash"];
    assert_eq!(
        next_token_index(hashes.iter().copied(), Some("first-hash")),
        1
    );
    assert_eq!(
        next_token_index(hashes.iter().copied(), Some("second-hash")),
        0
    );
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig {
        cases: 128,
        ..proptest::prelude::ProptestConfig::default()
    })]

    #[test]
    fn rotation_selects_the_successor_for_arbitrary_non_empty_pools(
        pool_size in 1usize..128,
        prior_index in 0usize..128,
    ) {
        let hashes: Vec<String> = (0..pool_size)
            .map(|index| format!("token-{index}"))
            .collect();
        let last_hash = hashes.get(prior_index).map(String::as_str);
        let expected = if prior_index < pool_size {
            (prior_index + 1) % pool_size
        } else {
            0
        };

        proptest::prop_assert_eq!(
            next_token_index(hashes.iter().map(String::as_str), last_hash),
            expected,
        );
    }
}
