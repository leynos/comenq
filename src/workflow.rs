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
    let Some(goreleaser) = jobs.get("goreleaser") else {
        return Ok(false);
    };
    let Some(steps) = goreleaser.get("steps") else {
        return Ok(false);
    };
    let Some(arr) = steps.as_sequence() else {
        return Ok(false);
    };
    for step in arr {
        if let Some(uses) = step.get("uses")
            && uses
                .as_str()
                .is_some_and(|s| s.contains("goreleaser-action"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "simplify test output")]
    use super::uses_goreleaser;

    #[test]
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
    fn missing_goreleaser() {
        let yaml = "jobs: {}";
        assert!(!uses_goreleaser(yaml).expect("parse"));
    }
}
