"""Ask GNU make itself which goal a makefile settles on.

Both the reader contracts and the bounded property checks pin ``default_goal``
to this oracle, so it lives here once and they cannot drift apart.
"""

from __future__ import annotations

import os
import subprocess


def make_default_goal(makefile: str, gnu_make: str) -> str | None:
    """Return the default goal GNU make itself settles on, or ``None``.

    ``make -pn`` prints the variable database without running a recipe, and
    ``.DEFAULT_GOAL`` is the value make settled on after reading every
    assignment (GNU make manual, "Other Special Variables"). ``None`` means
    make refused the makefile or printed no such variable.
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
