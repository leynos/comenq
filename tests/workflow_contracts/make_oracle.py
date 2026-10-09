"""Ask GNU make itself which goal a makefile settles on.

Both the reader contracts and the bounded property checks pin ``default_goal``
to this oracle. The question is split from the process that answers it:
``parse_default_goal`` and ``make_default_goal`` are pure over a
``MakeDatabase`` runner, and ``run_make_database`` is the one place that starts
``make``. A test takes the runner from the ``make_database`` fixture, so the
environment make runs in is chosen at the test boundary and a stand-in runner
can drive the failure paths without a process.
"""

from __future__ import annotations

import dataclasses as dc
import subprocess
import typing as typ


@dc.dataclass(frozen=True)
class MakeOutput:
    """What one ``make`` run printed and how it ended."""

    returncode: int
    stdout: str
    stderr: str


class MakeDatabase(typ.Protocol):
    """Run ``make -pn`` over a makefile read from standard input."""

    def __call__(self, makefile: str) -> MakeOutput:
        """Return the output of ``make -f - -pn`` fed ``makefile``."""
        ...


@dc.dataclass(frozen=True)
class Goal:
    """The default goal make settled on (empty when it settled on none)."""

    value: str


@dc.dataclass(frozen=True)
class Refused:
    """Make would not give an answer, with why."""

    returncode: int
    reason: str


def run_make_database(
    makefile: str, gnu_make: str, env: typ.Mapping[str, str]
) -> MakeOutput:
    """Start ``gnu_make -f - -pn`` in ``env`` and capture what it prints.

    ``-pn`` prints the variable database without running a recipe. A missing
    binary raises, so an absent make is never read as a refused makefile.
    """
    result = subprocess.run(  # noqa: S603 - a fixed command and a fixture
        [gnu_make, "-f", "-", "-pn"],
        input=makefile,
        capture_output=True,
        text=True,
        check=False,
        env=dict(env),
    )
    return MakeOutput(result.returncode, result.stdout, result.stderr)


def parse_default_goal(stdout: str) -> str | None:
    """Return the value of ``.DEFAULT_GOAL`` in a variable database, if printed."""
    for line in stdout.splitlines():
        name, _, value = line.replace(" := ", " = ").partition(" = ")
        if name == ".DEFAULT_GOAL":
            return value
    return None


def make_default_goal(makefile: str, database: MakeDatabase) -> Goal | Refused:
    """Return the default goal make settles on, or why it gave none.

    ``.DEFAULT_GOAL`` is the value make settled on after reading every
    assignment (GNU make manual, "Other Special Variables").
    """
    output = database(makefile)
    if output.returncode:
        return Refused(output.returncode, output.stderr.strip())
    goal = parse_default_goal(output.stdout)
    if goal is None:
        return Refused(output.returncode, "make printed no .DEFAULT_GOAL")
    return Goal(goal)
