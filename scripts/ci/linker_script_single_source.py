#!/usr/bin/env python3
"""Exactly one place may pass `-Tlink.x` for a firmware crate.

`cortex-m-rt`'s `link.x` pulls in the `memory.x` a firmware crate's build.rs
writes. Include it twice and the MEMORY block is defined twice, and the link
dies with:

    rust-lld: error: .../out/memory.x:10: region 'FLASH' already defined

This has now happened twice, both times the same way: a crate's build.rs
started passing its own `-Tlink.x` (so that a plain
`cargo build -p <crate> --target <t>` links on its own, instead of producing an
ELF with entry point 0x0 that the simulator rejects at step 0) while some
caller kept its `RUSTFLAGS="-C link-arg=-Tlink.x"` prefix.

It hides well. Cargo does not relink a cached artifact, so on a warm `target/`
the doubled flag costs nothing; the break appears only on a cold build. In
2026-08 that meant core-ci was green on the self-hosted runners for a week and
went red the first time the job landed on a clean hosted one.

So this derives the rule instead of restating it: any crate whose build.rs
emits `cargo:rustc-link-arg=-Tlink.x` owns that flag, and no workflow or script
may pass it again for that crate. Nothing here is a hardcoded list — add a
build.rs line and the callers are checked from the next run on.

    python3 scripts/ci/linker_script_single_source.py --check
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

BUILD_RS_EMIT = re.compile(r'cargo:rustc-link-arg=-Tlink\.x')
PASSES_LINK_X = re.compile(r'-Tlink\.x')
# `-p crate`, `-p=crate` and `--package crate`, quoted or not.
PACKAGE_ARG = re.compile(r'(?:-p|--package)[=\s]+"?\'?([A-Za-z0-9_.-]+)')


BLOCK_COMMENT = re.compile(r"/\*.*?\*/", re.DOTALL)


def _is_comment(line: str) -> bool:
    stripped = line.lstrip()
    return stripped.startswith("#") or stripped.startswith("//")


def _without_block_comments(text: str) -> str:
    """Blank out `/* ... */` spans, keeping line numbers intact.

    A build.rs that carries its emit inside a block comment is not an owner.
    Getting this wrong points the gate at a crate that passes nothing, and the
    message it prints would then be false in both halves.
    """
    return BLOCK_COMMENT.sub(lambda m: re.sub(r"[^\n]", " ", m.group(0)), text)


def self_linking_crates(root: Path) -> dict[str, Path]:
    """Crates whose own build.rs passes `-Tlink.x`."""
    found: dict[str, Path] = {}
    for build_rs in sorted(root.glob("crates/*/build.rs")):
        text = _without_block_comments(
            build_rs.read_text(encoding="utf-8", errors="replace")
        )
        for line in text.splitlines():
            if _is_comment(line):
                continue
            if BUILD_RS_EMIT.search(line):
                found[build_rs.parent.name] = build_rs.relative_to(root)
                break
    return found


def _scanned_files(root: Path) -> list[Path]:
    files: list[Path] = []
    workflows = root / ".github" / "workflows"
    if workflows.is_dir():
        files.extend(sorted(workflows.glob("*.yml")))
        files.extend(sorted(workflows.glob("*.yaml")))
    scripts = root / "scripts"
    if scripts.is_dir():
        files.extend(sorted(scripts.rglob("*.sh")))
    return files


def violations(root: Path) -> list[str]:
    owners = self_linking_crates(root)
    if not owners:
        return []

    out: list[str] = []
    for path in _scanned_files(root):
        text = path.read_text(encoding="utf-8", errors="replace")
        for number, line in enumerate(text.splitlines(), start=1):
            if _is_comment(line) or not PASSES_LINK_X.search(line):
                continue
            for crate in PACKAGE_ARG.findall(line):
                if crate in owners:
                    rel = path.relative_to(root)
                    out.append(
                        f"{rel}:{number}: passes -Tlink.x for `{crate}`, which "
                        f"already passes its own from {owners[crate]}. Two "
                        f"copies of link.x duplicate memory.x's MEMORY block "
                        f'("region \'FLASH\' already defined") on any cold '
                        f"build. Drop the flag here: `cargo build -p {crate} "
                        f"--release --target <target>` links on its own."
                    )
    return out


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero when a caller re-passes a crate's own -Tlink.x",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=REPO_ROOT,
        help="repository root to scan (default: this checkout)",
    )
    args = parser.parse_args(argv)

    owners = self_linking_crates(args.root)
    found = violations(args.root)

    if found:
        print("-Tlink.x is passed twice:\n", file=sys.stderr)
        for line in found:
            print(f"  {line}\n", file=sys.stderr)
        return 1

    print(
        f"-Tlink.x single source: OK "
        f"({len(owners)} crate(s) pass their own, no caller repeats it)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
