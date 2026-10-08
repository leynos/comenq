"""Contract tests for the ``RUSTFLAGS`` of the CI steps that call cargo directly.

The Makefile recipes and the ``setup-rust`` steps are held by the build
standard's Rust contract (``tests/build_standard_contract.rs``). Two steps in
``ci.yml`` call cargo themselves, and an assigned ``RUSTFLAGS`` replaces every
``rustflags`` table in ``.cargo/config.toml``, so each assigns its own value in
its own ``env`` block:

* ``Install Merman CLI`` builds on the isolated stable 1.95.0 toolchain, which
  must never see the nightly-only ``-Zthreads`` flag or the mold linker flag,
  so it denies warnings and nothing else.
* ``Test and Measure Coverage`` is a measurement, so it takes neither standard
  flag either: ``-D warnings`` and nothing else.

The workflow is parsed with PyYAML, so a step is judged by its own mapping, not
by a text window around it, and an assignment on a sibling step, on the job or
on the workflow does not count. Fixtures come first, so the rule cannot pass by
finding no steps.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml

CI_PATH = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"

#: The job whose steps are judged.
JOB = "build-test"

#: Step name -> the exact ``RUSTFLAGS`` value it must assign in its own ``env``.
EXPECTED_RUSTFLAGS = {
    "Install Merman CLI": "-D warnings",
    "Test and Measure Coverage": "-D warnings",
}


def problems(workflow: dict[str, object]) -> list[str]:
    """Return the complaints about a parsed workflow.

    Parameters
    ----------
    workflow : dict[str, object]
        The parsed ``ci.yml``.

    Returns
    -------
    list[str]
        One complaint per step that is missing, or that does not assign exactly
        the expected ``RUSTFLAGS`` in its own ``env``, and one per job or
        workflow ``env`` that assigns it for every step.
    """
    found: list[str] = []
    jobs = workflow.get("jobs")
    job = jobs.get(JOB) if isinstance(jobs, dict) else None
    if not isinstance(job, dict):
        return [f"the workflow has no {JOB!r} job"]
    for scope, env in (("workflow", workflow.get("env")), ("job", job.get("env"))):
        if isinstance(env, dict) and "RUSTFLAGS" in env:
            found.append(f"{scope}-level env assigns RUSTFLAGS, which every step would inherit")
    steps = [step for step in job.get("steps", []) if isinstance(step, dict)]
    for name, expected in EXPECTED_RUSTFLAGS.items():
        named = [step for step in steps if step.get("name") == name]
        if len(named) != 1:
            found.append(f"expected one step named {name!r}, found {len(named)}")
            continue
        env = named[0].get("env")
        value = env.get("RUSTFLAGS") if isinstance(env, dict) else None
        if value != expected:
            found.append(f"step {name!r} assigns RUSTFLAGS={value!r}, not {expected!r}")
    return found


def _workflow(steps: list[dict[str, object]], **extra: object) -> dict[str, object]:
    """Build a one-job workflow around the given steps."""
    return {"jobs": {JOB: {"steps": steps, **extra}}}


def _compliant_steps() -> list[dict[str, object]]:
    """Return steps that satisfy the contract."""
    return [
        {"name": name, "env": {"RUSTFLAGS": value}, "run": "cargo test"}
        for name, value in EXPECTED_RUSTFLAGS.items()
    ]


def test_the_real_workflow_holds_the_contract() -> None:
    """The repository's own ``ci.yml`` satisfies the contract."""
    workflow = yaml.safe_load(CI_PATH.read_text(encoding="utf-8"))
    assert problems(workflow) == []


def test_a_compliant_fixture_is_accepted() -> None:
    """Steps that assign exactly the expected value pass."""
    assert problems(_workflow(_compliant_steps())) == []


@pytest.mark.parametrize(
    ("label", "workflow"),
    [
        (
            "no env on the Merman step",
            _workflow(
                [{"name": "Install Merman CLI", "run": "cargo install"}, _compliant_steps()[1]]
            ),
        ),
        (
            "the frontend flag on the Merman step",
            _workflow(
                [
                    {
                        "name": "Install Merman CLI",
                        "env": {"RUSTFLAGS": "-D warnings -Zthreads=8"},
                    },
                    _compliant_steps()[1],
                ]
            ),
        ),
        (
            "the linker on the coverage step",
            _workflow(
                [
                    _compliant_steps()[0],
                    {
                        "name": "Test and Measure Coverage",
                        "env": {"RUSTFLAGS": "-D warnings -Clink-arg=-fuse-ld=mold"},
                    },
                ]
            ),
        ),
        (
            "an extra non-standard flag on the coverage step",
            _workflow(
                [
                    _compliant_steps()[0],
                    {
                        "name": "Test and Measure Coverage",
                        "env": {"RUSTFLAGS": "-D warnings -C target-cpu=native"},
                    },
                ]
            ),
        ),
        (
            "the assignment on a sibling step",
            _workflow(
                [
                    {"name": "Install Merman CLI", "run": "cargo install"},
                    {"name": "Sibling", "env": {"RUSTFLAGS": "-D warnings"}},
                    _compliant_steps()[1],
                ]
            ),
        ),
        (
            "the assignment on the job instead of the step",
            _workflow(
                [
                    {"name": "Install Merman CLI", "run": "cargo install"},
                    {"name": "Test and Measure Coverage", "run": "make"},
                ],
                env={"RUSTFLAGS": "-D warnings"},
            ),
        ),
        ("a step that is gone", _workflow([_compliant_steps()[0]])),
        ("no such job", {"jobs": {}}),
    ],
)
def test_a_defective_fixture_is_refused(label: str, workflow: dict[str, object]) -> None:
    """Each way of losing or widening an assignment draws a complaint."""
    assert problems(workflow), label


def test_a_workflow_level_assignment_is_refused() -> None:
    """A workflow-wide ``RUSTFLAGS`` reaches every step, including the stable one."""
    workflow = _workflow(_compliant_steps())
    workflow["env"] = {"RUSTFLAGS": "-D warnings"}
    assert any("workflow-level" in complaint for complaint in problems(workflow))
