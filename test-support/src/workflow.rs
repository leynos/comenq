//! Utilities for inspecting GitHub workflow files.

use serde_yaml::Value;

/// The prefix for the shared release build composite action identifier.
const RUST_BUILD_RELEASE_PREFIX: &str = "leynos/shared-actions/.github/actions/rust-build-release@";

/// The prefix for the shared release asset upload composite action identifier.
const UPLOAD_RELEASE_ASSETS_PREFIX: &str =
    "leynos/shared-actions/.github/actions/upload-release-assets@";

/// Return `true` when `commit` is a full 40-character lowercase hex commit SHA.
///
/// Dependabot owns the specific SHA value each `uses:` ref is pinned to, so
/// this checks only the *shape* of the pin (a commit SHA, not a mutable
/// branch or tag such as `main`), not which commit it names.
fn is_forty_hex_commit_sha(commit: &str) -> bool {
    commit.len() == 40
        && commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Return `true` when the release workflow uses the shared composite actions to
/// build binaries and publish packages, each pinned to a full commit SHA.
///
/// Dependabot bumps `rust-build-release` and `upload-release-assets`
/// independently, so their pinned commits are not required to match one
/// another — only that each is a genuine 40-hex commit SHA on the correct
/// path.
///
/// # Errors
///
/// Returns an error if the YAML cannot be parsed.
pub fn uses_shared_release_actions(yaml: &str) -> Result<bool, serde_yaml::Error> {
    let doc: Value = serde_yaml::from_str(yaml)?;
    let Some(jobs) = doc.get("jobs") else {
        return Ok(false);
    };
    let Some(map) = jobs.as_mapping() else {
        return Ok(false);
    };

    let mut saw_rust_builder = false;
    let mut saw_release_publisher = false;
    for job in map.values() {
        let Some(steps) = job.get("steps") else {
            continue;
        };
        let Some(arr) = steps.as_sequence() else {
            continue;
        };
        for step in arr {
            if let Some(uses) = step.get("uses").and_then(Value::as_str) {
                if uses
                    .strip_prefix(RUST_BUILD_RELEASE_PREFIX)
                    .is_some_and(is_forty_hex_commit_sha)
                {
                    saw_rust_builder = true;
                }
                if uses
                    .strip_prefix(UPLOAD_RELEASE_ASSETS_PREFIX)
                    .is_some_and(is_forty_hex_commit_sha)
                {
                    saw_release_publisher = true;
                }
            }
        }
    }

    Ok(saw_rust_builder && saw_release_publisher)
}

#[cfg(test)]
mod tests {
    use super::uses_shared_release_actions;
    use rstest::rstest;

    const RUST_BUILDER: &str = "leynos/shared-actions/.github/actions/rust-build-release@cb06757ebba47bb018ac0ade84fa5dc9ffb95020";
    const UPLOAD_RELEASE_ASSETS: &str = "leynos/shared-actions/.github/actions/upload-release-assets@cb06757ebba47bb018ac0ade84fa5dc9ffb95020";

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn detects_shared_actions() {
        let yaml = format!(
            r#"
        jobs:
          release:
            steps:
              - uses: {RUST_BUILDER}
              - uses: {UPLOAD_RELEASE_ASSETS}
        "#
        );
        assert!(uses_shared_release_actions(&yaml).expect("parse"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn independently_pinned_commits_are_accepted() {
        // Dependabot bumps one action's pin at a time, so the two actions
        // legitimately land on different commits between bumps.
        let yaml = format!(
            r#"
        jobs:
          release:
            steps:
              - uses: {RUST_BUILDER}
              - uses: leynos/shared-actions/.github/actions/upload-release-assets@deadbeefdeadbeefdeadbeefdeadbeefdeadbeef
        "#
        );
        assert!(uses_shared_release_actions(&yaml).expect("parse"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn missing_builder_fails() {
        let yaml = format!(
            r#"
        jobs:
          release:
            steps:
              - uses: {UPLOAD_RELEASE_ASSETS}
        "#
        );
        assert!(!uses_shared_release_actions(&yaml).expect("parse"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn missing_publisher_fails() {
        let yaml = format!(
            r#"
        jobs:
          release:
            steps:
              - uses: {RUST_BUILDER}
        "#
        );
        assert!(!uses_shared_release_actions(&yaml).expect("parse"));
    }

    #[rstest]
    #[case::unpinned_builder(
        "leynos/shared-actions/.github/actions/rust-build-release@v1",
        UPLOAD_RELEASE_ASSETS
    )]
    #[case::unpinned_publisher(
        RUST_BUILDER,
        "leynos/shared-actions/.github/actions/upload-release-assets@v1"
    )]
    #[case::short_builder_commit(
        "leynos/shared-actions/.github/actions/rust-build-release@cb06757",
        UPLOAD_RELEASE_ASSETS
    )]
    #[case::uppercase_publisher_commit(
        RUST_BUILDER,
        "leynos/shared-actions/.github/actions/upload-release-assets@CB06757EBBA47BB018AC0ADE84FA5DC9FFB95020"
    )]
    fn invalid_action_pinning_fails(#[case] builder: &str, #[case] publisher: &str) {
        let yaml = format!(
            r#"
        jobs:
          release:
            steps:
              - uses: {builder}
              - uses: {publisher}
        "#
        );
        assert!(!uses_shared_release_actions(&yaml).expect("parse"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn malformed_yaml_errors() {
        let yaml = "jobs: [";
        _ = uses_shared_release_actions(yaml).expect_err("expected parse failure");
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn missing_jobs_returns_false() {
        let yaml = r"
        name: release
        ";
        assert!(!uses_shared_release_actions(yaml).expect("parse"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn missing_steps_returns_false() {
        let yaml = r"
        jobs:
          release:
            name: publish
        ";
        assert!(!uses_shared_release_actions(yaml).expect("parse"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn non_sequence_steps_returns_false() {
        let yaml = r"
        jobs:
          release:
            steps:
              uses: leynos/shared-actions/.github/actions/rust-build-release@7bc9b6c15964ef98733aa647b76d402146284ba3
        ";
        assert!(!uses_shared_release_actions(yaml).expect("parse"));
    }
}
