//! Utilities for inspecting GitHub workflow files.

use serde_yaml::Value;

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
                    == "leynos/shared-actions/.github/actions/rust-build-release@7bc9b6c15964ef98733aa647b76d402146284ba3"
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
    use super::uses_shared_release_actions;

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn detects_shared_actions() {
        let yaml = r"
        jobs:
          release:
            steps:
              - uses: leynos/shared-actions/.github/actions/rust-build-release@7bc9b6c15964ef98733aa647b76d402146284ba3
              - uses: softprops/action-gh-release@v2
        ";
        assert!(uses_shared_release_actions(yaml).expect("parse"));
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
        let yaml = r"
        jobs:
          release:
            steps:
              - uses: leynos/shared-actions/.github/actions/rust-build-release@7bc9b6c15964ef98733aa647b76d402146284ba3
        ";
        assert!(!uses_shared_release_actions(yaml).expect("parse"));
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
