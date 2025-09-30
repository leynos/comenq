//! Utilities for inspecting GitHub workflow files.

use serde_yaml::Value;

/// The expected commit hash for the shared-actions repository.
const EXPECTED_SHARED_ACTIONS_COMMIT: &str = "1479e2ffbbf1053bb0205357dfe965299b7493ed";

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
    let expected_rust_builder = format!(
        "leynos/shared-actions/.github/actions/rust-build-release@{EXPECTED_SHARED_ACTIONS_COMMIT}",
    );
    for job in map.values() {
        let Some(steps) = job.get("steps") else {
            continue;
        };
        let Some(arr) = steps.as_sequence() else {
            continue;
        };
        for step in arr {
            if let Some(uses) = step.get("uses").and_then(Value::as_str) {
                if uses == expected_rust_builder {
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

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn detects_shared_actions() {
        let yaml = format!(
            r#"
        jobs:
          release:
            steps:
              - uses: leynos/shared-actions/.github/actions/rust-build-release@{}
              - uses: softprops/action-gh-release@v2
        "#,
            EXPECTED_SHARED_ACTIONS_COMMIT,
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
              - uses: leynos/shared-actions/.github/actions/rust-build-release@{}
        "#,
            EXPECTED_SHARED_ACTIONS_COMMIT,
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
