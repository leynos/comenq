"""Properties of the suite-once reader, over generated command lines and Makefiles.

The reader classifies shell text, so the cases in ``suite_runs_once_test.py``
pin named spellings. These properties state what must hold for every
combination the reader promises to read: a suite command is found however it
is joined and prefixed, a harmless command never is, and the default goal is
what applying the ``.DEFAULT_GOAL`` assignments in order leaves. The default
goal is also compared with real GNU make over every bounded sequence of
assignments, so make, not a second reading of its manual, is the oracle.
"""

from __future__ import annotations

import itertools

import pytest
from hypothesis import given
from hypothesis import strategies as st
from make_oracle import Goal, MakeDatabase, make_default_goal
from suite_commands import default_goal, runs_suite

SUITE_COMMANDS = (
    "make test",
    "make",
    "cargo test",
    "cargo nextest run",
    "pytest",
    "python -m pytest",
)
HARMLESS_COMMANDS = (
    "make lint",
    "echo ok",
    "cargo build",
    "make -n test",
    "pytest --help",
)
JOINERS = (";", " ; ", "&&", " || ", " | ", "\n", "&")
PREFIXES = (
    "",
    "X=1 ",
    "env X=1 ",
    "timeout 5m ",
    "then ",
    "do ",
    "nohup ",
    "sudo -u ci ",
)
#: A bare ``make`` runs the default goal, so the properties fix one that runs
#: the suite and read the bare case against it.
SUITE_GOAL = "test"


@given(
    first=st.sampled_from(HARMLESS_COMMANDS),
    joiner=st.sampled_from(JOINERS),
    prefix=st.sampled_from(PREFIXES),
    command=st.sampled_from(SUITE_COMMANDS),
)
def test_a_suite_run_is_found_however_it_is_joined_and_prefixed(
    first: str, joiner: str, prefix: str, command: str
) -> None:
    """Find a suite command after any harmless one, behind any prefix.

    Parameters
    ----------
    first, joiner, prefix, command : str
        The harmless command, the joiner, the prefix and the suite command.

    Returns
    -------
    None
        The test passes when the composed line is read as a suite run.
    """
    line = f"{first}{joiner}{prefix}{command}"
    assert runs_suite(line, SUITE_GOAL), f"a suite run was missed in {line!r}"


@given(
    first=st.sampled_from(HARMLESS_COMMANDS),
    joiner=st.sampled_from(JOINERS),
    prefix=st.sampled_from(PREFIXES),
    second=st.sampled_from(HARMLESS_COMMANDS),
)
def test_a_harmless_command_is_never_read_as_a_suite_run(
    first: str, joiner: str, prefix: str, second: str
) -> None:
    """Never read two harmless commands, joined and prefixed, as a suite run.

    Parameters
    ----------
    first, joiner, prefix, second : str
        The two harmless commands, the joiner and the prefix.

    Returns
    -------
    None
        The test passes when the composed line is not read as a suite run.
    """
    line = f"{first}{joiner}{prefix}{second}"
    assert not runs_suite(line, SUITE_GOAL), f"{line!r} was read as a suite run"


@given(word=st.text(alphabet="ab ;&|\"\\\n#$", max_size=12))
def test_quoted_text_never_adds_a_suite_run(word: str) -> None:
    """Keep a suite command quoted as text from counting, whatever surrounds it.

    A single-quoted string is one word to the shell, so ``echo '<anything>'``
    runs only ``echo``.

    Parameters
    ----------
    word : str
        Arbitrary text, without a single quote, placed inside the quotes.

    Returns
    -------
    None
        The test passes when the line is not read as a suite run.
    """
    line = f"echo '{word} make test {word}'"
    assert not runs_suite(line, SUITE_GOAL), f"{line!r} was read as a suite run"


#: One operation on ``.DEFAULT_GOAL``: an operator and a value.
OPERATIONS = st.tuples(
    st.sampled_from([":=", "=", "?=", "+="]), st.sampled_from(["", "a", "b", "test"])
)


def _reference(operations: list[tuple[str, str]]) -> str | None:
    """Return ``.DEFAULT_GOAL`` after the operations, applied as make applies them."""
    goal = ""
    for operator, value in operations:
        if operator in {":=", "="}:
            goal = value
        elif operator == "+=":
            goal = f"{goal} {value}".strip()
    return goal if len(goal.split()) == 1 else None


def _makefile(operations: list[tuple[str, str]]) -> str:
    """Return a Makefile that applies the operations, then declares its rules."""
    lines = [f".DEFAULT_GOAL {operator} {value}" for operator, value in operations]
    return "\n".join([*lines, "build:", "a:", "b:", "test:", ""])


@given(st.lists(OPERATIONS, max_size=4))
def test_the_default_goal_is_what_the_assignments_leave(
    operations: list[tuple[str, str]],
) -> None:
    """Match a reference fold of the assignments, or the first rule.

    Parameters
    ----------
    operations : list of tuple of str
        The operator and value of each ``.DEFAULT_GOAL`` assignment.

    Returns
    -------
    None
        The test passes when the reader agrees with the reference model.
    """
    expected = _reference(operations) or "build"
    makefile = _makefile(operations)
    assert default_goal(makefile) == expected, makefile


def test_every_bounded_assignment_sequence_agrees_with_gnu_make(
    make_database: MakeDatabase,
) -> None:
    """Compare the reader with make over every sequence of up to three assignments.

    The space is small enough to enumerate (the operators and values above,
    zero to three long), so this is exhaustive, not sampled, and make is the
    oracle.

    Parameters
    ----------
    make_database : MakeDatabase
        The runner of ``make -pn``, from the shared fixture.

    Returns
    -------
    None
        The test passes when the reader agrees with make on every sequence.
    """
    options = [
        (op, value) for op in (":=", "=", "?=", "+=") for value in ("", "a", "test")
    ]
    disagreements = []
    for length in range(4):
        for operations in itertools.product(options, repeat=length):
            makefile = _makefile(list(operations))
            by_make = make_default_goal(makefile, make_database)
            if not isinstance(by_make, Goal):
                continue
            if default_goal(makefile) != by_make.value:
                disagreements.append((operations, by_make.value, default_goal(makefile)))
    assert not disagreements, f"the reader disagrees with make on {disagreements[:5]}"


@pytest.mark.parametrize("command", SUITE_COMMANDS)
def test_every_suite_command_is_found_alone(command: str) -> None:
    """Find each suite command on its own line.

    Parameters
    ----------
    command : str
        A suite command.

    Returns
    -------
    None
        The test passes when the command is read as a suite run.
    """
    assert runs_suite(command, SUITE_GOAL), f"{command!r} was not read as a suite run"
