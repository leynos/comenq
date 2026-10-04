"""Contract that a pull request runs the test suite once, in the coverage step.

`make test` runs the nextest suite over the whole workspace, with every target
and every feature, and then the cucumber harness. The coverage step in
`ci.yml`'s `build-test` job used to run a narrower nextest suite (no
`--all-features`, no `--all-targets`) after a step running `make test`, so the
suite ran twice on every pull request. The coverage step now selects the same
scope, so it is the one place the suite runs. Only the cucumber harness, which
has its own `main` that nextest cannot execute, keeps a step of its own, behind
the `make test-cucumber` target.

The workflows are read by what their steps invoke, not by their names, through
`suite_commands.py`, which splits a command the way the shell does. This module
holds the split from both sides: no pull-request step but the cucumber step runs
the suite, and the coverage step and the cucumber step both really run.

Run via ``make test-workflow-contracts``.
"""

from __future__ import annotations

import os
import pathlib
import subprocess
import typing as typ

import pytest
import yaml
from suite_commands import default_goal, runs_suite

REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOWS = REPOSITORY_ROOT / ".github" / "workflows"
MAKEFILE = REPOSITORY_ROOT / "Makefile"
COVERAGE_ACTION = "leynos/shared-actions/.github/actions/generate-coverage@"
CUCUMBER_COMMAND = "make test-cucumber"
SUITE_JOB = "build-test"
#: The coverage inputs that give the run `make test`'s scope.
SCOPE_INPUTS = {"all-features": "true", "all-targets": "true"}
#: Inputs that would narrow the run below that scope.
NARROWING_INPUTS = ("features", "with-default-features", "cargo-manifest")
#: The default goal the spelling cases assume: one that runs the suite.
SUITE_GOAL = "all"


def _makefile_goal() -> str:
    """Return the goal a bare ``make`` runs here, from the Makefile."""
    return default_goal(MAKEFILE.read_text(encoding="utf-8"))


def _triggers(document: dict[str, typ.Any]) -> list[str]:
    """Return a workflow's trigger names, whatever form ``on`` takes.

    PyYAML reads an unquoted ``on`` key as the boolean ``True``.
    """
    on = document.get(True, document.get("on"))
    if isinstance(on, dict):
        return list(on)
    return [on] if isinstance(on, str) else list(on or [])


def _pull_request_steps() -> list[tuple[str, str, dict[str, typ.Any]]]:
    """Return ``(workflow, job, step)`` for every pull-request workflow step."""
    found = []
    for path in sorted(WORKFLOWS.glob("*.y*ml")):
        document = yaml.safe_load(path.read_text(encoding="utf-8"))
        if "pull_request" not in _triggers(document):
            continue
        for job, body in (document.get("jobs") or {}).items():
            found.extend((path.name, job, step) for step in body.get("steps") or [])
    return found


def _suite_job() -> dict[str, typ.Any]:
    """Return the ``build-test`` job of ``ci.yml`` as written, metadata included."""
    document = yaml.safe_load((WORKFLOWS / "ci.yml").read_text(encoding="utf-8"))
    return document["jobs"][SUITE_JOB]


def test_the_suite_job_runs_on_every_pull_request() -> None:
    """Refuse a job-level condition that would skip the whole suite job.

    The steps are held unconditional elsewhere, but a job-level ``if`` skips
    every step at once, so the suite would run zero times, not once.

    Returns
    -------
    None
        The test passes when ``build-test`` carries no ``if:`` of its own.
    """
    job = _suite_job()
    assert "if" not in job, (
        f"`{SUITE_JOB}` must run on every pull request, but it carries "
        f"`if: {job.get('if')}`, which can skip the suite entirely"
    )


def _suite_job_steps() -> list[dict[str, typ.Any]]:
    """Return the steps of ``ci.yml``'s ``build-test`` job."""
    return [
        step
        for name, job, step in _pull_request_steps()
        if (name, job) == ("ci.yml", SUITE_JOB)
    ]


def _coverage_steps() -> list[dict[str, typ.Any]]:
    """Return the ``build-test`` steps that use the coverage action."""
    return [
        step
        for step in _suite_job_steps()
        if COVERAGE_ACTION in str(step.get("uses", ""))
    ]


@pytest.mark.parametrize(
    ("command", "expected"),
    [
        ("make test", True),
        ("make test WITH_ACT=1", True),
        ("make -j2 test", True),
        ("make -C . test", True),
        ("make all", True),
        ("cargo nextest run --all-targets", True),
        ("cargo test --all-features", True),
        ("cargo --config tools/dev-fast/config.toml test", True),
        ("cargo +nightly test", True),
        ("cargo llvm-cov nextest --lcov", True),
        ("set -eu && make test", True),
        ("make", True),
        ('make "test"', True),
        ("make coverage", True),
        ("make lint&&make test", True),
        ("RUSTFLAGS='-D warnings' cargo test", True),
        ("env RUN_ACT_VALIDATION=1 make test", True),
        ("make \\\ntest", True),
        ("make lint # then\nmake test", True),
        ("make test-workflow-contracts", False),
        ("make lint", False),
        ("cargo build --all-targets", False),
        ("cargo run -- test", False),
        ("echo cargo test", False),
        ("echo 'pre;make test;post'", False),
        ("# make test", False),
        ("if true; then cargo test; fi", True),
        ("(cd crate && cargo test)", True),
        ("timeout 30m make test", True),
        ("bash -c 'cargo test'", True),
        ("NAME=foo#bar make test", True),
        ("make test#notes", False),
        ("bash scripts/check.sh", False),
        ("nohup cargo test", True),
        ("sudo -u ci make test", True),
        ("bash -lc 'make test'", True),
        ("sh -ec 'make test'", True),
        ("bash -c 'cargo' test", False),
        ("make -j 4", True),
        ("make -l 4", True),
        ("make -j test", True),
        ("make -j lint", False),
        ("make >suite.log", True),
        ("make >> suite.log 2>&1", True),
        ("make 2> err.log test", True),
        ("make test > out.log", True),
        ("make lint > suite.log", False),
        ('make ">x"', False),
        ("make < in.txt lint", False),
        ("command pytest -v", True),
        ("command -v pytest", False),
        ("pytest --collect-only", False),
        ("pytest --co -q", False),
        ("pytest --help", False),
        ("pytest --version", False),
        ("python -m pytest --fixtures", False),
        ("uv run pytest --markers", False),
        ("uvx pytest --collect-only", False),
        ("timeout 5m pytest --setup-plan", False),
        ("pytest -q tests", True),
        ("cargo te\\\nst", True),
        ("make\\\ntest", False),
        ("make -j 4 lint", False),
        ("make --jobs 4", True),
        ('echo "a \\" ; make test"', False),
        ('echo "a \\" b" ; make test', True),
        ("pytest tests", True),
        ("py.test", True),
        ("python -m pytest", True),
        ("python3.13 -m pytest", True),
        ("uvx pytest", True),
        ("uv run pytest", True),
        ("uv run --with pytest python -m pytest", True),
        ("python script.py", False),
        ("make dev-test", True),
        ("make test-fast", True),
        ("echo 'make test", False),
        ('make "test', True),
        ("", False),
        ("&", False),
        ("make -n", False),
        ("make -ns", False),
        ("make --help", False),
        ("command -v make", False),
        ("uv run --directory . cargo test", True),
        ("echo ok # ; make test", False),
    ],
)
def test_the_suite_pattern(command: str, *, expected: bool) -> None:
    """Recognize every spelling of a suite run, and nothing longer."""
    assert runs_suite(command, SUITE_GOAL) is expected, command


@pytest.mark.parametrize(
    "prefix",
    [
        "env -u HOME",
        "timeout -s KILL 5m",
        "nice -n 5",
        "command",
        "exec -a name",
        "nohup",
        "setsid",
        "stdbuf -oL",
        "sudo -u ci",
        "uv run --directory .",
    ],
)
def test_every_wrapper_is_looked_through(prefix: str) -> None:
    """Look through each wrapper, with an option of its own, to its command."""
    assert runs_suite(f"{prefix} make test", SUITE_GOAL), prefix
    assert not runs_suite(f"{prefix} make lint", SUITE_GOAL), prefix


@pytest.mark.parametrize(
    ("goal", "expected"), [("build", False), ("all", True), ("test", True)]
)
def test_a_bare_make_runs_the_default_goal(goal: str, *, expected: bool) -> None:
    """Read a bare ``make`` as a suite run only when the default goal is one."""
    assert runs_suite("make", goal) is expected


@pytest.mark.parametrize(
    ("makefile", "expected"),
    [
        (".PHONY: a\nbuild: x\nall: y\n", "build"),
        (".DEFAULT_GOAL := test\nbuild:\n", "test"),
        (".DEFAULT_GOAL ?= test\nbuild:\n", "build"),
        (".DEFAULT_GOAL += test\nbuild:\n", "test"),
        (".DEFAULT_GOAL = test\nbuild:\n", "test"),
        (".DEFAULT_GOAL := first\n.DEFAULT_GOAL := second\nbuild:\n", "second"),
        (".DEFAULT_GOAL := first\n.DEFAULT_GOAL ?= second\nbuild:\n", "first"),
        (".DEFAULT_GOAL ?= second\n.DEFAULT_GOAL := first\nbuild:\n", "first"),
        (".DEFAULT_GOAL := build\nlint:\n.DEFAULT_GOAL := test\n", "test"),
        (".DEFAULT_GOAL := first\n.DEFAULT_GOAL :=\nbuild:\n", "build"),
        (".DEFAULT_GOAL := first\n.DEFAULT_GOAL += second\nbuild:\n", "build"),
        (".DEFAULT_GOAL   :=   spaced\nbuild:\n", "spaced"),
        ("first:\n\t.DEFAULT_GOAL = test\n", "first"),
        ("# build: not a rule\nrun: z\n", "run"),
        (".PHONY: a\n.SUFFIXES:\nrun: z\n", "run"),
        ("X := 1\n\tfoo: bar\n%.o: %.c\nrun: z\n", "run"),
        ("X := 1\n", ""),
    ],
)
def test_the_default_goal_is_read(makefile: str, expected: str) -> None:
    """Read the default goal the way make does."""
    assert default_goal(makefile) == expected


@pytest.mark.parametrize(
    "target", ["test", "all", "coverage", "dev-test", "test-fast", "test-cucumber"]
)
def test_every_suite_target_runs_the_suite(target: str) -> None:
    """Read each suite target as a suite run, and a longer name as none."""
    assert runs_suite(f"make {target}", "build")
    assert runs_suite(f"make -j 4 {target}", "build")
    assert not runs_suite(f"make {target}-not", "build")


@pytest.mark.parametrize(
    "option",
    [
        "--just-print",
        "--dry-run",
        "--recon",
        "-n",
        "--question",
        "-q",
        "--help",
        "--version",
        "-v",
        "-ns",
    ],
)
def test_every_inert_make_option_runs_no_goal(option: str) -> None:
    """Refuse to read a make option that runs no goal as a suite run."""
    assert not runs_suite(f"make {option}", "all")
    assert not runs_suite(f"make {option} test", "all")
    assert not runs_suite(f"make test {option}", "all")


@pytest.mark.parametrize(
    "line", ["command -v make test", "command -V make test", "command -V make"]
)
def test_command_lookup_runs_nothing(line: str) -> None:
    """Read `command -v` and `command -V` as running nothing."""
    assert not runs_suite(line, "all")


def _make_default_goal(makefile: str, gnu_make: str) -> str | None:
    """Return the default goal GNU make itself settles on, or ``None``.

    ``make -pn`` prints the variable database without running a recipe, and
    ``.DEFAULT_GOAL`` is the value make settled on after reading every
    assignment (GNU make manual, "Other Special Variables").
    """
    result = subprocess.run(  # noqa: S603 - a fixed command and a fixture
        [gnu_make, "-f", "-", "-pn"],
        input=makefile,
        capture_output=True,
        text=True,
        check=False,
        env={"PATH": os.environ["PATH"]},
    )
    if result.returncode:
        return None
    for line in result.stdout.splitlines():
        name, _, value = line.replace(" := ", " = ").partition(" = ")
        if name == ".DEFAULT_GOAL":
            return value
    return None


MAKE_FIXTURES = [
    ".PHONY: a\nbuild: x\nx:\n",
    ".DEFAULT_GOAL := test\nbuild:\ntest:\n",
    ".DEFAULT_GOAL ?= test\nbuild:\ntest:\n",
    ".DEFAULT_GOAL = test\nbuild:\ntest:\n",
    ".DEFAULT_GOAL := first\n.DEFAULT_GOAL := second\nfirst:\nsecond:\nbuild:\n",
    ".DEFAULT_GOAL := first\n.DEFAULT_GOAL ?= second\nfirst:\nsecond:\n",
    ".DEFAULT_GOAL ?= second\n.DEFAULT_GOAL := first\nfirst:\nsecond:\n",
    ".DEFAULT_GOAL := build\nbuild:\ntest:\n.DEFAULT_GOAL := test\n",
    ".DEFAULT_GOAL := first\n.DEFAULT_GOAL :=\nbuild:\nfirst:\n",
    ".DEFAULT_GOAL += test\nbuild:\ntest:\n",
    "# build: not a rule\nrun:\n",
    ".PHONY: a\n.SUFFIXES:\nrun:\n",
    "first:\n\t@: .DEFAULT_GOAL = test\nsecond:\n",
]


@pytest.mark.parametrize("makefile", MAKE_FIXTURES)
def test_the_reader_agrees_with_gnu_make(makefile: str, gnu_make: str) -> None:
    """Pin the default-goal reader to make itself, not to a reading of its manual."""
    by_make = _make_default_goal(makefile, gnu_make)
    assert by_make is not None, "make must accept the fixture"
    assert default_goal(makefile) == by_make, makefile


def test_make_refuses_several_words_and_the_reader_does_not_read_them(
    gnu_make: str,
) -> None:
    """Fall back to the first rule where make refuses a multi-word goal."""
    makefile = (
        ".DEFAULT_GOAL := first\n.DEFAULT_GOAL += second\nbuild:\nfirst:\nsecond:\n"
    )
    assert _make_default_goal(makefile, gnu_make) is None
    assert default_goal(makefile) == "build"


def _is_the_cucumber_step(name: str, job: str, step: dict[str, typ.Any]) -> bool:
    """Report whether a step is the one expected cucumber step of ``build-test``.

    ``make test-cucumber`` is a suite run to the reader, so this step is exempted
    here, by name of workflow, job and exact command, and nowhere else: a second
    such step, or one in another workflow, still fails the test.
    """
    return (name, job) == ("ci.yml", SUITE_JOB) and str(
        step.get("run", "")
    ).strip() == CUCUMBER_COMMAND


def test_the_suite_runs_only_in_the_coverage_step() -> None:
    """Refuse any pull-request step that runs the suite outside coverage."""
    goal = _makefile_goal()
    repeated = [
        (name, job, str(step.get("run", "")).strip())
        for name, job, step in _pull_request_steps()
        if runs_suite(str(step.get("run", "")), goal)
        and not _is_the_cucumber_step(name, job, step)
    ]
    assert not repeated, f"the suite runs outside coverage in {repeated!r}"


def test_coverage_runs_once_and_unconditionally() -> None:
    """Require exactly one coverage step in ``build-test``, with no ``if:``."""
    steps = _coverage_steps()
    assert len(steps) == 1, f"{SUITE_JOB} must run the coverage action once"
    assert "if" not in steps[0], "coverage must run on every pull request"


def test_coverage_measures_the_scope_make_test_runs() -> None:
    """Require every target and every feature, and no narrowing input."""
    inputs = _coverage_steps()[0].get("with") or {}
    assert {key: inputs.get(key) for key in SCOPE_INPUTS} == SCOPE_INPUTS
    narrowed = [key for key in NARROWING_INPUTS if key in inputs]
    assert not narrowed, f"coverage is narrowed by {narrowed}"


UPLOAD_ACTION = "leynos/shared-actions/.github/actions/upload-codescene-coverage@"


def _step_index(
    steps: list[dict[str, typ.Any]], matches: typ.Callable[[dict[str, typ.Any]], bool]
) -> int:
    """Return the position of the one step ``matches`` selects, or fail."""
    found = [index for index, step in enumerate(steps) if matches(step)]
    assert len(found) == 1, f"expected exactly one matching step, found {len(found)}"
    return found[0]


def test_the_cucumber_step_runs_after_the_coverage_upload() -> None:
    """Hold the order: coverage, then the CodeScene upload, then cucumber.

    A failing scenario must not stop the coverage or its upload, so the
    cucumber step has to come after both.

    Returns
    -------
    None
        The test passes when the three steps are in that order.
    """
    steps = _suite_job_steps()
    coverage = _step_index(
        steps, lambda step: COVERAGE_ACTION in str(step.get("uses", ""))
    )
    upload = _step_index(steps, lambda step: UPLOAD_ACTION in str(step.get("uses", "")))
    cucumber = _step_index(
        steps, lambda step: str(step.get("run", "")).strip() == CUCUMBER_COMMAND
    )
    assert coverage < upload < cucumber, (
        "`build-test` must run coverage, then the CodeScene upload, then "
        f"`{CUCUMBER_COMMAND}`; their positions are {coverage}, {upload} and {cucumber}"
    )


def test_the_cucumber_harness_runs_in_its_own_step() -> None:
    """Require one unconditional step running ``make test-cucumber``."""
    steps = [
        step
        for step in _suite_job_steps()
        if str(step.get("run", "")).strip() == CUCUMBER_COMMAND
    ]
    assert len(steps) == 1, f"{SUITE_JOB} must run `{CUCUMBER_COMMAND}` once"
    assert "if" not in steps[0], "the cucumber step must run on every pull request"


def _recipe(target: str) -> list[str]:
    """Return the recipe lines of a Makefile target."""
    lines = MAKEFILE.read_text(encoding="utf-8").splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith(f"{target}:"))
    recipe = []
    for line in lines[start + 1 :]:
        if not line.startswith("\t"):
            break
        recipe.append(line.strip())
    return recipe


def test_the_cucumber_target_runs_the_harness_over_the_workspace() -> None:
    """Require ``--test cucumber`` with the workspace and every feature."""
    recipe = " ".join(_recipe("test-cucumber"))
    assert "$(CARGO) test --workspace --all-features --test cucumber" in recipe


def test_make_test_still_runs_nextest_and_the_cucumber_target() -> None:
    """Keep the local `make test` running both halves of the old suite."""
    recipe = " ".join(_recipe("test"))
    assert "nextest run --workspace --all-targets --all-features" in recipe
    assert "test-cucumber" in recipe


def _dry_run(target: str, gnu_make: str) -> list[str]:
    """Return the commands ``make`` would run for a target, in order.

    ``make -n`` prints each recipe line after expansion without running it, so
    an ``echo``-only or commented-out recipe shows up as what it is. Directory
    notices from the recursive ``$(MAKE)`` call are dropped.
    """
    result = subprocess.run(  # noqa: S603 - a fixed command on a fixed target
        [gnu_make, "-n", target, "CARGO=cargo", "BUILD_JOBS="],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        check=True,
        env={"PATH": os.environ["PATH"]},
    )
    # `$(MAKE)` expands to the path make was started by, so name it `make`.
    return [
        line.strip().replace(f"{gnu_make} ", "make ", 1)
        for line in result.stdout.splitlines()
        if line.strip() and not line.startswith("make[")
    ]


def test_make_test_runs_nextest_then_the_cucumber_target(gnu_make: str) -> None:
    """Assert the commands and order `make test` would run, not the recipe text.

    Parameters
    ----------
    gnu_make : str
        The path of a GNU make, from the shared fixture.

    Returns
    -------
    None
        The test passes when the expanded commands match, in order.
    """
    commands = _dry_run("test", gnu_make)
    assert commands == [
        'RUSTFLAGS="-D warnings" cargo nextest run --workspace --all-targets --all-features',
        "make test-cucumber",
        'RUSTFLAGS="-D warnings" cargo test --workspace --all-features --test cucumber',
    ], (
        "`make test` must run nextest over every target and feature, then "
        f"`make test-cucumber`, then the harness; it would run {commands!r}"
    )


def test_make_test_cucumber_runs_the_harness_over_the_workspace(gnu_make: str) -> None:
    """Assert the one command `make test-cucumber` would run.

    Parameters
    ----------
    gnu_make : str
        The path of a GNU make, from the shared fixture.

    Returns
    -------
    None
        The test passes when the single expanded command matches.
    """
    commands = _dry_run("test-cucumber", gnu_make)
    assert commands == [
        'RUSTFLAGS="-D warnings" cargo test --workspace --all-features --test cucumber'
    ], (
        "`make test-cucumber` must run only the cucumber harness over the "
        f"workspace with every feature; it would run {commands!r}"
    )


def test_the_cucumber_target_is_declared_phony() -> None:
    """Require `test-cucumber` among the `.PHONY` targets, so a file of that name cannot hide it.

    Returns
    -------
    None
        The test passes when the target is declared phony.
    """
    declared = next(
        line
        for line in MAKEFILE.read_text(encoding="utf-8").splitlines()
        if line.startswith(".PHONY:")
    )
    assert "test-cucumber" in declared.split(), (
        "the Makefile's `.PHONY` declaration must list `test-cucumber`, so a "
        f"file of that name cannot stop the target running; it declares {declared!r}"
    )
