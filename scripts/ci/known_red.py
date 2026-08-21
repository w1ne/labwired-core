#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Read the `known_red` list out of scripts/ci/workspace-test-shards.json.

ONE home for "which tests are acknowledged red". `pr-workspace-tests` already
read this list through workspace_test_shard.py; `core-full` did not, and ran a
plain `cargo test --workspace`. So one acknowledged failure —
`cpu_trace_conformance::every_cpu_core_is_covered_by_this_file`, tracked as
issue 961 — was tolerated by one lane and fatal to the other, which is why
core-full had never once gone green.

That is not merely an ugly badge. `nightly_only_excluded` sends work TO
core-full. While core-full cannot pass, moving a slow test there does not
relocate it, it deletes it.

Emitting the list from here rather than restating it in YAML is the point: two
lanes cannot drift into disagreeing about what is known-red.
"""

import argparse
import json
import pathlib
import sys

CONFIG = pathlib.Path(__file__).with_name("workspace-test-shards.json")


def entries() -> list[dict]:
    return json.loads(CONFIG.read_text()).get("known_red", [])


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    group = ap.add_mutually_exclusive_group(required=True)
    group.add_argument(
        "--skip-args",
        action="store_true",
        help="one libtest argument per line: --skip, then the test name",
    )
    group.add_argument(
        "--rows",
        action="store_true",
        help="one 'package target test issue' row per line, for a shell loop",
    )
    args = ap.parse_args()

    for entry in entries():
        if args.skip_args:
            # Two lines, not "--skip NAME": the caller reads this into a bash
            # array, and a single line would arrive as one argv element that
            # libtest rejects.
            print("--skip")
            print(entry["test"])
        else:
            print(entry["package"], entry["target"], entry["test"], entry.get("issue", "-"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
