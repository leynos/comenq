//! Utilities for inspecting GitHub workflow files.

use serde_yaml::Value;

/// The commit hash for the shared-actions repository that the workflow must be pinned to.
const SHARED_ACTIONS_COMMIT: &str = "1479e2ffbbf1053bb0205357dfe965299b7493ed";

#[cfg(test)]
/// The expected commit hash for the shared-actions repository.
const EXPECTED_SHARED_ACTIONS_COMMIT: &str = SHARED_ACTIONS_COMMIT;
/// The prefix for the shared release build composite action identifier.
const RUST_BUILD_RELEASE_PREFIX: &str = "leynos/shared-actions/.github/actions/rust-build-release@";

/// Return `true` when the release workflow uses the shared composite actions to
/// build binaries and publish packages.
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
                    .is_some_and(|commit| commit == SHARED_ACTIONS_COMMIT)
                {
                    saw_rust_builder = true;
                }
                if uses.starts_with("softprops/action-gh-release@") {
                    saw_release_publisher = true;
                }
            }
        }
    }

    Ok(saw_rust_builder && saw_release_publisher)
}

#[cfg(test)]
mod tests {
    use super::{EXPECTED_SHARED_ACTIONS_COMMIT, uses_shared_release_actions};

    fn builder_action() -> String {
        format!(
            "leynos/shared-actions/.github/actions/rust-build-release@{}",
            EXPECTED_SHARED_ACTIONS_COMMIT,
        )
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn detects_shared_actions() {
        let builder = builder_action();
        let yaml = format!(
            r#"
        jobs:
          release:
            steps:
              - uses: {}
              - uses: softprops/action-gh-release@v2
        "#,
            builder,
        );
        assert!(uses_shared_release_actions(&yaml).expect("parse"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn missing_builder_fails() {
        let yaml = r"
        jobs:
          release:
            steps:
              - uses: softprops/action-gh-release@v2
        ";
        assert!(!uses_shared_release_actions(yaml).expect("parse"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn missing_publisher_fails() {
        let yaml = format!(
            r#"
        jobs:
          release:
            steps:
              - uses: {}
        "#,
            builder_action(),
        );
        assert!(!uses_shared_release_actions(&yaml).expect("parse"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn mismatched_builder_commit_fails() {
        let yaml = r#"
        jobs:
          release:
            steps:
              - uses: leynos/shared-actions/.github/actions/rust-build-release@deadbeefdeadbeefdeadbeefdeadbeefdeadbeef
              - uses: softprops/action-gh-release@v2
        "#;
        assert!(!uses_shared_release_actions(yaml).expect("parse"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn unpinned_builder_fails() {
        let yaml = r#"
        jobs:
          release:
            steps:
              - uses: leynos/shared-actions/.github/actions/rust-build-release@v1
              - uses: softprops/action-gh-release@v2
        "#;
        assert!(!uses_shared_release_actions(yaml).expect("parse"));
    }
}
