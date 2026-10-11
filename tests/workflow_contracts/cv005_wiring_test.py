"""Contract for the wiring of the shared CV-005 contract check.

The CV-005 clauses live in ``leynos/shared-actions`` (``cv005-contracts``) and
the library's own suite proves them. What the library cannot prove is that this
repository calls it: that the Makefile names a full commit, runs
``check --repository .`` under the Python the library needs, that
``.github/cv005.toml`` names this repository, that ``make all`` includes the
target, and that CI runs it. Each of those is read here by running ``make -n``
and parsing the workflow, so removing or misspelling any of them fails a test.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

import re
import subprocess
import tomllib
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
REPOSITORY = "leynos/comenq"
TARGET = "test-workflow-contracts"
SOURCE = (
    "git+https://github.com/leynos/shared-actions@{ref}"
    "#subdirectory=packages/cv005-contracts"
)
FULL_COMMIT = re.compile(r"[0-9a-f]{40}")
PIN = re.compile(r"^CV005_CONTRACTS_REF \?= (\S+)$", re.MULTILINE)


def _make_n(gnu_make: str, env: dict[str, str], target: str, *overrides: str) -> str:
    """Return the commands ``make -n TARGET [OVERRIDE=...]`` would run.

    The ``make`` executable and its environment are injected by the caller (the
    ``gnu_make`` and ``make_env`` fixtures), so no helper reads the ambient ones.
    """
    result = subprocess.run(  # noqa: S603 - a fixed command
        [gnu_make, "-n", target, *overrides],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout


def _pinned_commit() -> str:
    """Return the commit ``CV005_CONTRACTS_REF`` names in the Makefile."""
    match = PIN.search((ROOT / "Makefile").read_text("utf-8"))
    assert match is not None, "the Makefile must set CV005_CONTRACTS_REF"
    return match[1]


def test_the_pin_is_a_full_commit() -> None:
    """Refuse a branch, tag or abbreviated commit as the checker's source."""
    pin = _pinned_commit()
    assert FULL_COMMIT.fullmatch(pin), f"{pin!r} is not a full commit hash"


def test_the_target_runs_the_pinned_checker_on_this_repository(gnu_make: str, make_env: dict[str, str]) -> None:
    """Run ``check --repository .`` from the pinned source under Python 3.13."""
    commands = _make_n(gnu_make, make_env, TARGET)
    source = SOURCE.format(ref=_pinned_commit())
    runs = [line for line in commands.splitlines() if "cv005-contracts" in line]
    assert len(runs) == 1, f"expected one checker invocation, got {runs!r}"
    run = runs[0]
    assert "uv tool run" in run, run
    assert "--python 3.13" in run, run
    assert f"--from '{source}'" in run, run
    assert run.rstrip().endswith("cv005-contracts check --repository ."), run


def test_the_repository_parameter_names_this_repository() -> None:
    """Hold ``.github/cv005.toml`` to this repository's name."""
    config = tomllib.loads((ROOT / ".github" / "cv005.toml").read_text("utf-8"))
    assert config.get("repository") == REPOSITORY, config


def test_make_all_includes_the_target(gnu_make: str, make_env: dict[str, str]) -> None:
    """Run the checker from the comprehensive gate as well as on its own."""
    assert "cv005-contracts check" in _make_n(gnu_make, make_env, "all")


def test_ci_runs_the_target_unconditionally() -> None:
    """Require a CI step that runs the target under no step or job condition."""
    workflow = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "ci.yml").read_text("utf-8")
    )
    runs = [
        (job, step)
        for job in workflow["jobs"].values()
        for step in job.get("steps", [])
        if f"make {TARGET}" in str(step.get("run", ""))
    ]
    assert runs, f"ci.yml must run `make {TARGET}` in a step"
    assert all("if" not in step for _, step in runs), runs
    assert all("if" not in job for job, _ in runs), "the job carries an `if`"


def test_both_contract_runs_use_the_configured_uv(
    gnu_make: str, make_env: dict[str, str]
) -> None:
    """Route the checker and the pytest run through ``UV`` and ``UV_ENV``.

    A caller whose ``uv`` is not on ``PATH`` sets ``UV``; a run that calls a
    literal ``uv`` would ignore it.
    """
    output = _make_n(gnu_make, make_env, TARGET, "UV=/opt/injected/uv", "UV_ENV=INJECTED=1")
    runs = [
        line
        for line in output.splitlines()
        if "cv005-contracts" in line or "pytest" in line
    ]
    assert len(runs) == 2, runs
    assert all("INJECTED=1 /opt/injected/uv " in line for line in runs), runs
