#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Select measured PR admission from the actual tested merge, independent of labels."""
import argparse
import json
from pathlib import Path
import re
import subprocess

OWNED_REPOSITORY = "w1ne/labwired-core"
RUNTIME_PREFIXES = (
    "crates/cli/src/",
    "crates/config/src/",
    "crates/wasm/src/",
    "tests/fixtures/",
    "configs/chips/",
    "configs/systems/",
    "scripts/perf/",
    ".cargo/",
)
BUILD_FILES = {
    "Cargo.toml", "Cargo.lock", "rust-toolchain", "rust-toolchain.toml",
    ".github/workflows/core-perf.yml", ".github/workflows/core-perf-pr.yml",
    ".github/workflows/core-ci.yml",
    *(f"crates/{crate}/{name}" for crate in ("core", "cli", "config", "wasm")
      for name in ("Cargo.toml", "build.rs")),
}
FIRMWARE_PREFIXES = tuple(
    f"crates/firmware-perf-spin{suffix}/"
    for suffix in ("", "-avr", "-riscv", "-xtensa")
)


# A missing positive prefix must not silently exempt a new crate or build
# script. Only known documentation and independently compiled test trees skip.
# Firmware inputs under tests/fixtures are consumed by performance contracts.
INDEPENDENT_TEST_RE = re.compile(
    r"^(?:crates/(?:[^/]+/)*(?:tests|examples|benches)/"
    r"|crates/(?:[^/]+/)*src/tests/"
    r"|scripts/(?:[^/]+/)*test_[^/]+\.py$)"
)


def hotpath(path):
    if INDEPENDENT_TEST_RE.match(path):
        return False
    if path in BUILD_FILES:
        return True
    if path.endswith((".md", ".rst")):
        return False
    if path.startswith("docs/") and path.endswith(
        (".txt", ".svg", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".pdf")
    ):
        return False
    if path.startswith(RUNTIME_PREFIXES + FIRMWARE_PREFIXES):
        return True
    if path in {"LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE"}:
        return False
    if path.startswith("tests/") and not path.startswith("tests/fixtures/"):
        return False
    # Includes newly introduced production/build roots. Unknown is measured.
    return True


def select(event, repository, head_ref, paths):
    if event != "pull_request":
        return True, "manual/schedule measurement"
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
    if len(parents) != 2 or parents[1] != head:
        raise ValueError("tested merge must have exactly two parents and the exact PR head second")
    first_parent = parents[0]
    for sha in (base, first_parent, head):
        git("cat-file", "-e", sha + "^{commit}")
    try:
        git("merge-base", "--is-ancestor", base, first_parent)
    except subprocess.CalledProcessError as exc:
        if exc.returncode == 1:
            raise ValueError("tested merge first parent is not a descendant of the event base") from exc
        raise
    # Diff from the actual first parent to exclude unrelated later main changes.
    # No rename detection: removing a runtime path must still select the gate.
    raw = git("diff", "--name-only", "--no-renames", "-z", first_parent, tested)
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
    parser.add_argument("--perf-labeled", choices=("true", "false"), default="false")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    paths = changed_paths(args.tested, args.base, args.head) if args.event == "pull_request" else []
    required, reason = select(args.event, args.repository, args.head_ref, paths)
    # The label only requests the additional full matrix. It never overrides
    # the mandatory short measurement selected above.
    full_required = args.event != "pull_request" or (
        args.repository == OWNED_REPOSITORY and args.perf_labeled == "true"
    )
    # No output is written if source admission or Git fails.
    with args.output.open("a") as output:
        output.write(f"required={str(required).lower()}\n")
        output.write(f"full_required={str(full_required).lower()}\n")
    print(json.dumps(dict(required=required, full_required=full_required, reason=reason, tested=args.tested, base=args.base, head=args.head, paths=paths), sort_keys=True))


if __name__ == "__main__":
    main()
