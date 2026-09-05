"""Guard Comenq's repository-owned Namespace runner assignments."""

from pathlib import Path
import subprocess

import pytest
import yaml

_REPO_ROOT = Path(__file__).resolve().parents[2]
_EXPECTED_RUNNERS = (
    ("delayed-pr-comment.yml", "delay_and_comment"),
    ("release.yml", "metadata"),
    ("release.yml", "build-packages"),
    ("release.yml", "release"),
)
_NAMESPACE_RUNNER = "namespace-profile-default"


@pytest.mark.parametrize(("workflow_name", "job_name"), _EXPECTED_RUNNERS)
def test_repository_owned_linux_job_uses_shared_namespace_profile(
    workflow_name: str, job_name: str
) -> None:
    """Require each direct Linux job to retain its reviewed runner profile."""
    workflow_path = _REPO_ROOT / ".github" / "workflows" / workflow_name
    workflow = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
    assert workflow["jobs"][job_name]["runs-on"] == _NAMESPACE_RUNNER, (
        f"{workflow_name}:{job_name} must use {_NAMESPACE_RUNNER}"
    )


def test_ci_uses_github_hosted_linux_for_the_whitaker_toolchain() -> None:
    """Keep CI on a runner compatible with Whitaker's prebuilt cargo-dylint."""
    workflow_path = _REPO_ROOT / ".github" / "workflows" / "ci.yml"
    workflow = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
    assert workflow["jobs"]["build-test"]["runs-on"] == "ubuntu-latest"


@pytest.mark.parametrize(
    ("delay_minutes", "expected_seconds"),
    (("1", "60"), ("0", "0"), ("08", "480")),
)
def test_delayed_comment_calculates_seconds_for_valid_input(
    delay_minutes: str, expected_seconds: str, tmp_path: Path
) -> None:
    """Require the workflow's calculation step to preserve accepted delays."""
    workflow_path = _REPO_ROOT / ".github" / "workflows" / "delayed-pr-comment.yml"
    workflow = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
    calculation = workflow["jobs"]["delay_and_comment"]["steps"][0]["run"]
    output_path = tmp_path / "github-output"

    result = subprocess.run(
        ["bash", "-c", calculation],
        capture_output=True,
        check=False,
        env={"DELAY_MINUTES": delay_minutes, "GITHUB_OUTPUT": str(output_path)},
        text=True,
    )

    assert result.returncode == 0, result.stderr
    assert output_path.read_text(encoding="utf-8") == f"secs={expected_seconds}\n"


@pytest.mark.parametrize("delay_minutes", ("", "-1", "1.5", " 1", "$(id)"))
def test_delayed_comment_rejects_invalid_input(
    delay_minutes: str, tmp_path: Path
) -> None:
    """Require the workflow's calculation step to reject unsafe delay values."""
    workflow_path = _REPO_ROOT / ".github" / "workflows" / "delayed-pr-comment.yml"
    workflow = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
    calculation = workflow["jobs"]["delay_and_comment"]["steps"][0]["run"]
    output_path = tmp_path / "github-output"

    result = subprocess.run(
        ["bash", "-c", calculation],
        capture_output=True,
        check=False,
        env={"DELAY_MINUTES": delay_minutes, "GITHUB_OUTPUT": str(output_path)},
        text=True,
    )

    assert result.returncode != 0
    assert "delay_minutes must contain only digits" in result.stderr
    assert not output_path.exists()


@pytest.mark.parametrize("job_name", ("metadata", "build-packages"))
def test_non_publishing_release_jobs_retain_read_only_permissions(
    job_name: str,
) -> None:
    """Require non-publishing release jobs to retain their least privilege."""
    workflow_path = _REPO_ROOT / ".github" / "workflows" / "release.yml"
    workflow = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
    assert workflow["jobs"][job_name]["permissions"] == {"contents": "read"}
