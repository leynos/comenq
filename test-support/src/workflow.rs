//! Utilities for inspecting GitHub workflow files.

use serde_yaml::Value;

/// Return `true` if the workflow steps include the `GoReleaser` action.
///
/// # Errors
///
/// Returns an error if the YAML cannot be parsed.
pub fn uses_goreleaser(yaml: &str) -> Result<bool, serde_yaml::Error> {
    let doc: Value = serde_yaml::from_str(yaml)?;
    let Some(jobs) = doc.get("jobs") else {
        return Ok(false);
    };
    let Some(map) = jobs.as_mapping() else {
        return Ok(false);
    };
    for job in map.values() {
        let Some(steps) = job.get("steps") else {
            continue;
        };
        let Some(arr) = steps.as_sequence() else {
            continue;
        };
        for step in arr {
            if step
                .get("uses")
                .and_then(|u| u.as_str())
                .is_some_and(|s| s.starts_with("goreleaser/goreleaser-action"))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::uses_goreleaser;

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn detects_goreleaser() {
        let yaml = r"
        jobs:
          goreleaser:
            steps:
              - uses: goreleaser/goreleaser-action@v5
        ";
        assert!(uses_goreleaser(yaml).expect("parse"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "simplify test output")]
    fn missing_goreleaser() {
        let yaml = "jobs: {}";
        assert!(!uses_goreleaser(yaml).expect("parse"));
    }
}
