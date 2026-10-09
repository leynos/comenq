"""Shared pytest setup for the workflow contracts.

A contract that pins a reader to GNU make itself needs a GNU make on the host.
The direct ``uv run ... pytest`` entrypoint does not promise one, so those
contracts take the ``gnu_make`` fixture, which skips them with the reason when
``make`` is absent or is not GNU make, instead of failing on a missing binary.
"""

from __future__ import annotations

import functools
import os
import shutil
import subprocess

import pytest
from make_oracle import MakeDatabase, run_make_database


@pytest.fixture(scope="session")
def gnu_make() -> str:
    """Return the path of a GNU make, or skip the requesting test."""
    path = shutil.which("make")
    if path is None:
        pytest.skip("make is not installed, so the reader cannot be pinned to it")
    result = subprocess.run(  # noqa: S603 - a fixed command
        [path, "--version"], capture_output=True, text=True, check=False
    )
    if "GNU Make" not in result.stdout:
        pytest.skip("make is not GNU make, so the reader cannot be pinned to it")
    return path


@pytest.fixture
def make_env() -> dict[str, str]:
    """Return the environment make runs in: ``PATH`` and nothing else.

    It is read here, at the test boundary, so no helper reads it.
    """
    return {"PATH": os.environ["PATH"]}


@pytest.fixture
def make_database(gnu_make: str, make_env: dict[str, str]) -> MakeDatabase:
    """Return a runner of ``make -pn`` that sees only ``make_env``."""
    return functools.partial(run_make_database, gnu_make=gnu_make, env=make_env)
