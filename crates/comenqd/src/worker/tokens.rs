//! GitHub token clients, hashing, and rotation selection.

use octocrab::Octocrab;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::sync::Arc;

/// Constructs an authenticated Octocrab GitHub client using a personal access token.
#[expect(clippy::result_large_err, reason = "propagate Octocrab errors")]
pub(crate) fn build_octocrab(token: &str) -> octocrab::Result<Octocrab> {
    Octocrab::builder()
        .personal_token(token.to_string())
        .build()
}

/// Hex-encoded SHA-256 hash of a token value.
///
/// History records identify tokens by this hash rather than by name or
/// value, so rotation state survives token files being renamed and the log
/// never holds a secret.
///
/// # Examples
///
/// ```rust
/// let hash = comenqd::daemon::token_hash("s3cret");
/// assert_eq!(hash.len(), 64);
/// assert_eq!(hash, comenqd::daemon::token_hash("s3cret"));
/// ```
#[must_use]
pub fn token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// A GitHub client paired with its token's name and hash.
#[derive(Clone)]
pub struct TokenClient {
    /// Token file name, used only for logging.
    pub name: String,
    /// Hex SHA-256 of the token value, recorded in the history.
    pub hash: String,
    /// Client authenticated with the token.
    pub octocrab: Arc<Octocrab>,
}

impl TokenClient {
    /// Pair `octocrab` (authenticated with `token`) with the token's
    /// identifying name and hash.
    #[must_use]
    pub fn new(name: impl Into<String>, token: &str, octocrab: Arc<Octocrab>) -> Self {
        Self {
            name: name.into(),
            hash: token_hash(token),
            octocrab,
        }
    }
}

/// Index of the next token after the one represented by `last_hash`.
///
/// Empty history and hashes that no longer match the configured pool start
/// with the first token.
pub(super) fn next_token_index<'a>(
    mut token_hashes: impl ExactSizeIterator<Item = &'a str>,
    last_hash: Option<&str>,
) -> usize {
    let pool_size = token_hashes.len();
    if pool_size == 0 {
        return 0;
    }
    last_hash
        .and_then(|hash| token_hashes.position(|token_hash| token_hash == hash))
        .map_or(0, |index| (index + 1) % pool_size)
}
