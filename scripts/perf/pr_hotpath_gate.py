#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Select the unchanged full perf gate from the actual tested PR merge."""
import argparse
import json
from pathlib import Path
import re
import subprocess

OWNED_REPOSITORY = "w1ne/labwired-core"
RUNTIME_PREFIXES = (
    "crates/core/src/",
    "crates/cli/src/",
    "crates/config/src/",
    "configs/chips/",
    "configs/systems/",
    "scripts/perf/",
    ".cargo/",
)
BUILD_FILES = {
    "Cargo.toml", "Cargo.lock", "rust-toolchain", "rust-toolchain.toml",
    ".github/workflows/core-perf.yml",
    *(f"crates/{crate}/{name}" for crate in ("core", "cli", "config")
      for name in ("Cargo.toml", "build.rs")),
}
FIRMWARE_PREFIXES = tuple(
    f"crates/firmware-perf-spin{suffix}/"
    for suffix in ("", "-avr", "-riscv", "-xtensa")
)


def hotpath(path):
    # These separately compiled tests do not change the release engine.
    if path.startswith("crates/core/src/tests/"):
        return False
    return path in BUILD_FILES or path.startswith(RUNTIME_PREFIXES + FIRMWARE_PREFIXES)


def select(event, repository, head_ref, paths):
    if event != "pull_request":
        return True, "existing main/manual/schedule gate"
    if repository != OWNED_REPOSITORY:
        return False, "existing upstream PR policy"
    if head_ref.startswith(("merge/upstream-", "up/")):
        return True, "existing upstream-sync PR gate"
    return any(map(hotpath, paths)), "tested PR changed-path policy"


def git(*args):
    return subprocess.check_output(["git", *args])


def changed_paths(tested, base, head):
    for sha in (tested, base, head):
        if not re.fullmatch(r"[0-9a-f]{40}", sha):
            raise ValueError("full tested/base/head commit identities required")
    if git("rev-parse", "HEAD").decode().strip() != tested:
        raise ValueError("checkout differs from the event's tested commit")
    # cat-file retains real parents even at shallow depth; git-show %P does not.
    headers = git("cat-file", "commit", tested).split(b"\n\n", 1)[0].splitlines()
    parents = [line[7:].decode() for line in headers if line.startswith(b"parent ")]
    if parents != [base, head]:
        raise ValueError("tested merge must have the exact ordered base/head parents")
    for sha in (base, head):
        git("cat-file", "-e", sha + "^{commit}")
    # The first-parent diff excludes unrelated main changes on an older branch.
    # No rename detection: removing a runtime path must still select the gate.
    raw = git("diff", "--name-only", "--no-renames", "-z", base, tested)
    if raw and not raw.endswith(b"\0"):
        raise ValueError("incomplete changed-path stream")
    return [p.decode("utf-8", "surrogateescape") for p in raw.split(b"\0") if p]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--event", choices=("push", "pull_request", "schedule", "workflow_dispatch"), required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--head-ref", default="")
    parser.add_argument("--tested", required=True)
    parser.add_argument("--base", default="")
    parser.add_argument("--head", default="")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    paths = changed_paths(args.tested, args.base, args.head) if args.event == "pull_request" else []
    required, reason = select(args.event, args.repository, args.head_ref, paths)
    # No output is written if source admission or Git fails.
    with args.output.open("a") as output:
        output.write(f"required={str(required).lower()}\n")
    print(json.dumps(dict(required=required, reason=reason, tested=args.tested, base=args.base, head=args.head, paths=paths), sort_keys=True))


if __name__ == "__main__":
    main()
