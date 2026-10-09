"""Recognize shell commands that run the Rust or Python test suite.

The suite-once contract needs to know whether a workflow line runs the suite
outside the coverage action. A substring check is both too wide (``echo
pytest``) and too narrow (``make "test"``, ``make lint&&make test``), so a
command is split into segments at the shell separators that sit outside quotes,
escapes and comments. Each segment is split into words as the shell would, and
the program it runs is read before its arguments. Control words such as
``then`` and wrappers such as ``env``, ``timeout``, ``uv run`` and
``python -m`` are looked through, and the string after ``sh -c`` or
``bash -c`` is read as a command of its own.
"""

from __future__ import annotations

import itertools
import re
import shlex
from collections.abc import Callable
from pathlib import PurePosixPath

#: Make options that take their value as the next word.
MAKE_VALUE_OPTIONS = frozenset(
    {
        "-C",
        "-f",
        "-I",
        "-o",
        "-W",
        "--directory",
        "--file",
        "--makefile",
    }
)
#: Make options whose value is optional: it is read only when the next word is
#: a number, so ``make -j test`` still builds ``test``.
MAKE_NUMERIC_OPTIONS = frozenset({"-j", "-l", "--jobs", "--load-average", "--max-load"})
#: Make options that read or describe the makefile without running a goal.
MAKE_INERT_OPTIONS = frozenset(
    {"--just-print", "--dry-run", "--recon", "--question", "--help", "--version", "-v"}
)
#: Short make options that also run no goal, as they may be clustered (``-ns``).
MAKE_INERT_LETTERS = frozenset("nq")
#: Short make options that take a value, which ends a cluster of letters.
MAKE_VALUE_LETTERS = frozenset("CfIoWjl")
#: Make targets that run the suite: ``test``, ``coverage`` and the fast local
#: variants. ``all`` is not one: it builds the release binary and checks
#: spelling, and a test asserts that against what make would run. A bare
#: ``make`` runs the Makefile's default goal, so it counts only where that goal
#: is one of these.
SUITE_TARGETS = frozenset({"test", "coverage", "dev-test", "test-fast", "test-cucumber"})
#: ``uv run`` and ``uvx`` options that take their value as the next word.
UV_VALUE_OPTIONS = frozenset(
    {
        "--with",
        "--with-editable",
        "--with-requirements",
        "--python",
        "-p",
        "--group",
        "--extra",
        "--from",
        "--project",
        "--package",
        "--directory",
        "--env-file",
    }
)
#: Wrappers that run the command after their own options, mapped to the
#: options that take a value and the operands they read before the command.
WRAPPERS: dict[str, tuple[frozenset[str], int]] = {
    "env": (frozenset({"-u", "--unset", "-C", "--chdir"}), 0),
    "timeout": (frozenset({"-s", "--signal", "-k", "--kill-after"}), 1),
    "nice": (frozenset({"-n", "--adjustment"}), 0),
    "command": (frozenset(), 0),
    "exec": (frozenset({"-a"}), 0),
    "nohup": (frozenset(), 0),
    "setsid": (frozenset(), 0),
    "stdbuf": (frozenset({"-i", "-o", "-e", "--input", "--output", "--error"}), 0),
    "sudo": (
        frozenset(
            {"-u", "--user", "-g", "--group", "-h", "--host", "-p", "--prompt"}
            | {"-C", "-D", "--chdir", "-R", "--chroot", "-r", "--role", "-t", "--type"}
        ),
        0,
    ),
}
#: Reserved words that open or continue a compound command; the command
#: they introduce follows them in the same segment.
CONTROL_WORDS = frozenset(
    {"if", "then", "else", "elif", "do", "while", "until", "!", "{", "time"}
)
#: Shells whose ``-c`` operand is itself a command.
SHELLS = frozenset({"sh", "bash"})
#: The shell's view of a command, one piece at a time: a comment (dropped), a
#: line continuation (removed, without ending the word), a separator between commands, or text,
#: where quoted strings and escaped characters are kept whole so a separator
#: inside them does not split the command.
TOKENS = re.compile(
    r"""
    (?P<comment>(?:^|(?<=[\s;&|]))\#[^\n]*)
    |(?P<continuation>\\\n)
    |(?P<separator>[;&|\n])
    |(?P<text>'[^']*'|"(?:\\.|[^"\\])*"|\\.|[^'"\\;&|\n\#]+|\#)
    """,
    re.VERBOSE | re.DOTALL | re.MULTILINE,
)
#: A redirection outside quotes: an optional file descriptor, the operator and
#: its target. A quoted string or an escaped character is matched first and
#: kept, so ``make ">x"`` keeps its argument.
REDIRECTION = re.compile(
    r"""('[^']*'|"(?:\\.|[^"\\])*"|\\.)|\d*(?:>>|>|<)\s*(?:'[^']*'|"(?:\\.|[^"\\])*"|[^\s<>]*)""",
    re.DOTALL,
)
#: Python interpreters by name: ``python``, ``python3`` and ``python3.13``.
PYTHON_PROGRAM = re.compile(r"python(?:3(?:\.\d+)?)?")
#: pytest options that list or describe and run no test.
PYTEST_INERT_OPTIONS = frozenset(
    {
        "--collect-only",
        "--co",
        "--help",
        "-h",
        "--version",
        "-V",
        "--fixtures",
        "--fixtures-per-test",
        "--markers",
        "--setup-plan",
    }
)
#: Programs that are pytest itself.
PYTEST_PROGRAMS = frozenset({"pytest", "py.test"})
#: Cargo options that take their value as the next word.
CARGO_VALUE_OPTIONS = frozenset(
    {"--config", "-Z", "-C", "--manifest-path", "--color", "--target-dir"}
)
#: Cargo subcommands that run the suite.
SUITE_SUBCOMMANDS = frozenset({"test", "nextest", "llvm-cov"})


def _segments(command: str) -> list[str]:
    """Split a command at the separators outside quotes, escapes and comments."""
    segments = [""]
    for token in TOKENS.finditer(command):
        if token.lastgroup == "separator":
            segments.append("")
        elif token.lastgroup == "text":
            segments[-1] += token.group()
    return segments


def _is_assignment(word: str) -> bool:
    """Report whether a word is a leading ``NAME=value`` assignment."""
    return "=" in word and not word.startswith("-")


def _without_assignments(words: list[str]) -> list[str]:
    """Return the words from the first one that is not an assignment."""
    while words and _is_assignment(words[0]):
        words = words[1:]
    return words


def _words(segment: str) -> list[str]:
    """Split one segment into words as the shell would, less assignments.

    Comments were dropped when the command was segmented, so a ``#`` left here
    sits inside a word, as in ``make test#notes``, and stays part of it.
    """
    without_redirections = REDIRECTION.sub(lambda m: m.group(1) or " ", segment)
    try:
        words = shlex.split(without_redirections, comments=False)
    except ValueError:
        words = segment.split()
    if words:
        # A subshell's parentheses open its first segment and close its last.
        words[0] = words[0].lstrip("(")
        words[-1] = words[-1].rstrip(")")
    return _without_assignments([word for word in words if word])


def _program(words: list[str]) -> str:
    """Return the name of the program a word list runs, without its directory."""
    return PurePosixPath(words[0]).name if words else ""


def _operands(words: list[str], value_options: frozenset[str]) -> list[str]:
    """Return a command's operands: its words less options and their values."""
    found: list[str] = []
    skip = False
    for word in words:
        if skip:
            skip = False
        elif word in value_options:
            skip = True
        elif not word.startswith(("-", "+")) and "=" not in word:
            found.append(word)
    return found


def _after_options(words: list[str], value_options: frozenset[str]) -> list[str]:
    """Return the words from the first operand on."""
    index = 0
    while index < len(words) and words[index].startswith("-"):
        index += 2 if words[index] in value_options else 1
    return words[index:]


def _past_control_word(words: list[str]) -> list[str] | None:
    """Return the command after a leading reserved word such as ``then``."""
    return words[1:] if _program(words) in CONTROL_WORDS else None


def _is_lookup(arguments: list[str]) -> bool:
    """Report whether the options before the operand ask ``command`` to describe."""
    options = list(itertools.takewhile(lambda word: word.startswith("-"), arguments))
    return bool({"-v", "-V"} & set(options))


def _past_wrapper(words: list[str]) -> list[str] | None:
    """Return the command a wrapper such as ``env`` or ``timeout`` runs."""
    wrapper = WRAPPERS.get(_program(words))
    if wrapper is None:
        return None
    if _program(words) == "command" and _is_lookup(words[1:]):
        # ``command -v make`` describes ``make`` and runs nothing.
        return None
    value_options, leading_operands = wrapper
    after = _after_options(words[1:], value_options)[leading_operands:]
    return _without_assignments(after)


def _past_uv(words: list[str]) -> list[str] | None:
    """Return the command ``uv run`` or ``uvx`` runs."""
    program = _program(words)
    if program == "uv" and words[1:2] == ["run"]:
        return _after_options(words[2:], UV_VALUE_OPTIONS)
    if program == "uvx":
        return _after_options(words[1:], UV_VALUE_OPTIONS)
    return None


def _past_python_module(words: list[str]) -> list[str] | None:
    """Return the module command ``python -m`` runs."""
    is_module_run = words[1:2] == ["-m"]
    return (
        words[2:]
        if PYTHON_PROGRAM.fullmatch(_program(words)) and is_module_run
        else None
    )


#: Readers that each look through one kind of prefix to the command behind it.
UNWRAPPERS: tuple[Callable[[list[str]], list[str] | None], ...] = (
    _past_control_word,
    _past_wrapper,
    _past_uv,
    _past_python_module,
)


def _unwrap(words: list[str]) -> list[str]:
    """Strip every control word and wrapper in front of the command."""
    for unwrapper in UNWRAPPERS:
        inner = unwrapper(words)
        if inner is not None:
            return _unwrap(inner)
    return words


def _cargo_runs_suite(arguments: list[str], _default_goal: str) -> bool:
    """Report whether cargo's arguments name a suite-running subcommand."""
    operands = _operands(arguments, CARGO_VALUE_OPTIONS)
    return bool(operands) and operands[0] in SUITE_SUBCOMMANDS


def _is_number(word: str) -> bool:
    """Report whether a word is a number."""
    try:
        float(word)
    except ValueError:
        return False
    return True


def _takes_number(option: str, following: list[str]) -> bool:
    """Report whether a make option is followed by the number it takes."""
    return option in MAKE_NUMERIC_OPTIONS and bool(following) and _is_number(following[0])


def _make_operands(arguments: list[str]) -> list[str]:
    """Return make's operands: its words less options, values and assignments."""
    found: list[str] = []
    index = 0
    while index < len(arguments):
        word = arguments[index]
        following = arguments[index + 1 : index + 2]
        if word in MAKE_VALUE_OPTIONS:
            index += 1
        elif _takes_number(word, following):
            index += 1
        elif not word.startswith(("-", "+")) and "=" not in word:
            found.append(word)
        index += 1
    return found


def _is_inert_make_option(word: str) -> bool:
    """Report whether a make option stops make before it runs any goal."""
    if word in MAKE_INERT_OPTIONS:
        return True
    if not word.startswith("-") or word.startswith("--"):
        return False
    letters = []
    for letter in word[1:]:
        if not letter.isalpha() or letter in MAKE_VALUE_LETTERS:
            break
        letters.append(letter)
    return bool(MAKE_INERT_LETTERS & set(letters))


def _make_runs_suite(arguments: list[str], default_goal: str) -> bool:
    """Report whether make's arguments run a suite goal.

    A bare ``make`` runs the default goal, and an option such as ``-n`` stops
    make before it runs any goal.
    """
    if any(_is_inert_make_option(word) for word in arguments):
        return False
    targets = _make_operands(arguments) or [default_goal]
    return bool(SUITE_TARGETS & set(targets))


def _is_command_flag(word: str) -> bool:
    """Report whether a word is a short option group that includes ``c``."""
    return word.startswith("-") and not word.startswith("--") and "c" in word[1:]


def _shell_runs_suite(arguments: list[str], default_goal: str) -> bool:
    """Report whether ``sh -c`` or ``bash -c`` is given a suite-running command.

    Only the operand after the command flag is the script; later words are its
    positional parameters.
    """
    for index, word in enumerate(arguments):
        if _is_command_flag(word):
            script = arguments[index + 1 : index + 2]
            return bool(script) and runs_suite(script[0], default_goal)
    return False


def _pytest_runs_suite(arguments: list[str], _default_goal: str) -> bool:
    """Report whether pytest's arguments run tests and not just describe them."""
    return not PYTEST_INERT_OPTIONS & set(arguments)


#: What each suite-capable program's arguments must say for it to run the suite.
READERS: dict[str, Callable[[list[str], str], bool]] = {
    "cargo": _cargo_runs_suite,
    "make": _make_runs_suite,
    **dict.fromkeys(SHELLS, _shell_runs_suite),
    **dict.fromkeys(PYTEST_PROGRAMS, _pytest_runs_suite),
}


def _segment_runs_suite(segment: str, default_goal: str) -> bool:
    """Report whether one shell segment runs the suite."""
    words = _unwrap(_words(segment))
    reader = READERS.get(_program(words))
    return reader is not None and reader(words[1:], default_goal)


def _goal_assignment(line: str) -> tuple[str, str] | None:
    """Return ``(operator, value)`` for a line that assigns ``.DEFAULT_GOAL``."""
    name, separator, value = line.partition("=")
    name = name.rstrip()
    operator = ""
    if name.endswith(("+", "?")):
        operator, name = name[-1], name[:-1].rstrip()
    if separator and name.rstrip(":").rstrip() == ".DEFAULT_GOAL":
        return operator, value.strip()
    return None


def _apply_assignment(current: str | None, operator: str, value: str) -> str | None:
    """Return ``.DEFAULT_GOAL`` after one more assignment.

    ``=`` and ``:=`` replace it, ``?=`` changes nothing (make defines
    ``.DEFAULT_GOAL`` itself, empty, before it reads a makefile, so ``?=``
    finds it defined), ``+=`` appends a word, and an empty result clears it.
    """
    if operator == "?":
        result = current or ""
    elif operator == "+":
        result = f"{current or ''} {value}"
    else:
        result = value
    return result.strip() or None


def _assigned_goal(makefile: str) -> str | None:
    """Return what ``.DEFAULT_GOAL`` holds once every assignment has applied.

    A tab-indented line is recipe text, not an assignment. Make refuses a
    default goal of several targets, so such a value is not read.
    """
    goal: str | None = None
    for line in makefile.splitlines():
        assignment = None if line.startswith("\t") else _goal_assignment(line)
        if assignment:
            goal = _apply_assignment(goal, *assignment)
    return goal if goal and len(goal.split()) == 1 else None


def _rule_goal(line: str) -> str | None:
    """Return the first goal a rule line names, or ``None`` for other lines."""
    names, separator, rest = line.partition(":")
    is_rule = separator and not rest.startswith("=")
    is_special = line.startswith(("\t", " ", "#", ".")) or set("=%$") & set(names)
    words = names.split()
    return words[0] if is_rule and not is_special and words else None


def default_goal(makefile: str) -> str:
    """Return the goal a bare ``make`` runs, read from Makefile text.

    ``.DEFAULT_GOAL`` wins when set; otherwise the first rule that is not a
    special or pattern target is the default.

    Examples
    --------
    >>> default_goal(".PHONY: all\\nbuild: x\\nall: y\\n")
    'build'
    """
    assigned = _assigned_goal(makefile)
    if assigned is not None:
        return assigned
    goals = (_rule_goal(line) for line in makefile.splitlines())
    return next((goal for goal in goals if goal), "")


def runs_suite(command: str, default_goal: str) -> bool:
    """Report whether a shell command runs the suite, in any spelling.

    Parameters
    ----------
    command : str
        A workflow step's ``run`` text, which may span several lines.
    default_goal : str
        The goal a bare ``make`` runs, from the Makefile.

    Returns
    -------
    bool
        True when any segment runs pytest, ``cargo test``, ``cargo nextest``
        or ``cargo llvm-cov``, or runs ``make`` with a suite target or, with no
        target, when the default goal is one. Each may sit behind a control
        word, a wrapper or ``sh -c``.

    Examples
    --------
    >>> runs_suite("if true; then make test; fi", "build")
    True
    >>> runs_suite("echo 'pre;make test;post'", "build")
    False
    >>> runs_suite("make", "build")
    False
    """
    return any(_segment_runs_suite(part, default_goal) for part in _segments(command))
