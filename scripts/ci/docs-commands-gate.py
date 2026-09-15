#!/usr/bin/env python3
"""Every `labwired ...` command we print in user-facing docs must be a command.

The README told people to run

    labwired run --firmware path/to/firmware.elf --system configs/systems/<board>.yaml

for months. `labwired run` has never had a `--system` flag — it takes `--chip` —
so anyone who followed the "from your terminal" section got

    error: unexpected argument '--system' found

Nothing could have caught that: the docs were prose, and the CLI's flags live in
clap. This asks the binary itself what it accepts, which makes the CLI the one
source of truth and the docs the thing that has to keep up.

Usage: scripts/ci/docs-commands-gate.py [path-to-labwired]
"""
from __future__ import annotations

import re
import shlex
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# User-facing docs only. Plans and strategy notes under docs/superpowers and
# docs/strategy record what we were thinking at a point in time; they are not
# instructions anybody is invited to run.
INCLUDE = [
    REPO / "README.md",
    *(REPO / "docs").glob("*.md"),
    *(REPO / "docs" / "agent").rglob("*.md"),
    *(REPO / "docs" / "guides").rglob("*.md"),
    *(REPO / "docs" / "boards").rglob("*.md"),
]

COMMAND_RE = re.compile(r"^\s*(?:\$\s*)?(labwired(?:-dap)?\s+[^\n]*)$")
# `--flag`, `--flag=value`, but not a bare `--` or a markdown rule.
FLAG_RE = re.compile(r"^--[a-z0-9][a-z0-9-]*")


def cli_flags(cli: str, subcommand: str | None) -> set[str] | None:
    """Long flags clap advertises for a subcommand, or None if it has none."""
    argv = [cli] + ([subcommand] if subcommand else []) + ["--help"]
    try:
        out = subprocess.run(argv, capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.SubprocessError):
        return None
    if out.returncode != 0:
        return None
    return set(re.findall(r"(--[a-z0-9][a-z0-9-]*)", out.stdout))


def subcommands(cli: str) -> set[str]:
    out = subprocess.run([cli, "--help"], capture_output=True, text=True, timeout=60)
    found: set[str] = set()
    in_commands = False
    for line in out.stdout.splitlines():
        if re.match(r"^\s*Commands:", line):
            in_commands = True
            continue
        if in_commands:
            if re.match(r"^\s*(Options|Arguments):", line):
                break
            m = re.match(r"^\s{2,}([a-z][a-z0-9-]*)\s", line)
            if m:
                found.add(m.group(1))
    return found


def main() -> int:
    cli = sys.argv[1] if len(sys.argv) > 1 else "labwired"
    subs = subcommands(cli)
    if not subs:
        print(f"could not read subcommands from '{cli} --help'", file=sys.stderr)
        return 1

    flag_cache: dict[str | None, set[str] | None] = {}
    failures = 0
    checked = 0

    for path in sorted(set(INCLUDE)):
        if not path.is_file():
            continue
        rel = path.relative_to(REPO)
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            m = COMMAND_RE.match(line)
            if not m:
                continue
            raw = m.group(1).rstrip("\\").strip()
            # Placeholders like <board> are fine; the shell would choke on the
            # angle brackets, so strip them before splitting.
            cleaned = raw.replace("<", "").replace(">", "")
            try:
                argv = shlex.split(cleaned)
            except ValueError:
                continue
            if not argv:
                continue
            checked += 1

            # A subcommand, when there is one, is the FIRST word after the
            # program name. Anything else that is not a flag is a flag's value
            # — `--port 3333` does not mean there is a `3333` subcommand — and
            # the CLI also still accepts the legacy top-level form
            # (`labwired --firmware X --system Y`), where there is no
            # subcommand at all. Both had to be understood before this gate
            # could tell a broken instruction from an old-style one.
            sub = None
            if argv[0] == "labwired" and len(argv) > 1 and not argv[1].startswith("-"):
                if argv[1] not in subs:
                    print(f"{rel}:{lineno}: `{argv[1]}` is not a labwired subcommand")
                    print(f"    {raw}")
                    failures += 1
                    continue
                sub = argv[1]

            key = sub
            if key not in flag_cache:
                flag_cache[key] = cli_flags(cli, sub)
            allowed = flag_cache[key]
            if allowed is None:
                continue

            for token in argv[1:]:
                if not FLAG_RE.match(token):
                    continue
                flag = token.split("=", 1)[0]
                if flag not in allowed:
                    near = sorted(f for f in allowed if f[2:4] == flag[2:4])
                    hint = f" (did you mean {', '.join(near)}?)" if near else ""
                    print(f"{rel}:{lineno}: `labwired {sub or ''}` has no {flag}{hint}")
                    print(f"    {raw}")
                    failures += 1

    print()
    print(f"checked {checked} documented command(s) against {cli}")
    if failures:
        print(f"{failures} documented command(s) cannot run as written.")
        return 1
    print("every documented command is one the CLI accepts.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
