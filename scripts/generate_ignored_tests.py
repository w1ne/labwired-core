#!/usr/bin/env python3
"""Inventory every #[ignore]d Rust test, and stop new undocumented ones landing.

Why this exists
---------------
`cargo test` prints "N ignored" and moves on. Nothing recorded WHICH tests those
were or WHY, so the suite's real coverage was whatever the last person
remembered. At the time this was written the tree carried 148 `#[ignore]`
attributes across 66 files, and 68 of them had no reason string at all — a test
switched off with no record of who switched it off or what would turn it back
on. That is indistinguishable from a test that was quietly abandoned.

A skipped test reports as a passing suite. This repo has been bitten by that
shape repeatedly, so the ignores get a written, gated inventory.

What it does
------------
  (no args)   regenerate docs/testing/IGNORED_TESTS.md
  --check     fail if the committed doc is stale, or if the undocumented count
              exceeds the ratchet below

The ratchet only shrinks. Adding an `#[ignore]` WITH a reason is free — that is
a documented decision. Adding a bare `#[ignore]` fails the build, and the fix is
to write the reason, not to raise the number.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CRATES = REPO / "crates"
DOC = REPO / "docs" / "testing" / "IGNORED_TESTS.md"

# Ratchet. Lower it when you document an ignore; never raise it.
# 2026-08-14, core main fca5a5613: 114 `#[ignore]` attributes, 28 of them bare.
MAX_UNDOCUMENTED = 28

# Must match the attribute at the start of a line, so that prose mentioning
# `#[ignore]` in a doc comment is not counted — several files explain at length
# why something is or is not ignored, and counting those would inflate the
# inventory with commentary.
IGNORE_START_RE = re.compile(r"^\s*#\[ignore(?P<rest>.*)$")
REASON_ONE_LINE_RE = re.compile(r"^\s*=\s*\"(?P<reason>(?:[^\"\\]|\\.)*)\"\s*\]")
BARE_RE = re.compile(r"^\s*\]")
# The `#?` accepts `fn #hw_name()` inside a proc-macro `quote!` template.
# crates/hw-oracle-macros emits its ignored hw-oracle tests that way, so the
# name is interpolated at expansion time and there is no literal identifier to
# read. Those entries are marked rather than left blank — six of them showed up
# as `?` in the first run, which looks like a scanner bug instead of a real
# property of the code.
FN_RE = re.compile(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+(?P<name>#?[A-Za-z0-9_]+)")


def crate_of(path: Path) -> str:
    rel = path.relative_to(CRATES).parts
    return rel[0] if rel else "?"


def collect() -> list[dict]:
    """Every #[ignore], with the test it guards and its stated reason."""
    found = []
    for path in sorted(CRATES.rglob("*.rs")):
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        for i, line in enumerate(lines):
            m = IGNORE_START_RE.match(line)
            if not m:
                continue
            rest = m.group("rest")

            if BARE_RE.match(rest):
                reason = None
            else:
                one_line = REASON_ONE_LINE_RE.match(rest)
                if one_line:
                    reason = one_line.group("reason")
                else:
                    # A reason string continued across lines with a trailing
                    # backslash. Four of these existed when this was written and
                    # the first version of this scanner dropped them silently —
                    # an inventory that quietly omits entries is worse than none,
                    # so join the continuation rather than skipping the entry.
                    joined = rest
                    for probe in lines[i + 1 : i + 12]:
                        joined += " " + probe.strip()
                        if "\"]" in probe:
                            break
                    collapsed = re.sub(r"\\\s+", "", joined)
                    cont = REASON_ONE_LINE_RE.match(collapsed)
                    if cont:
                        reason = re.sub(r"\s+", " ", cont.group("reason")).strip()
                    else:
                        # Not an attribute we can read. Record it as undocumented
                        # rather than dropping it — it still switches a test off.
                        reason = None

            # The test name is the next `fn` — attributes may sit between the
            # #[ignore] and the signature (#[test], #[cfg(...)], #[should_panic]).
            name = "?"
            for probe in lines[i + 1 : i + 14]:
                fn = FN_RE.match(probe)
                if fn:
                    name = fn.group("name")
                    if name.startswith("#"):
                        name = f"{name} (macro-generated, one per hw-oracle target)"
                    break
            found.append(
                {
                    "crate": crate_of(path),
                    "file": str(path.relative_to(REPO)),
                    "line": i + 1,
                    "test": name,
                    "reason": reason,
                }
            )
    return found


def render(entries: list[dict]) -> str:
    undocumented = [e for e in entries if not e["reason"]]
    by_crate: dict[str, list[dict]] = defaultdict(list)
    for e in entries:
        by_crate[e["crate"]].append(e)

    out: list[str] = []
    out.append("# Ignored Rust tests")
    out.append("")
    out.append(
        "GENERATED by `scripts/generate_ignored_tests.py` — do not hand-edit. "
        "Run the script to refresh; `--check` fails the build when this file is stale."
    )
    out.append("")
    out.append(
        "`cargo test` reports ignored tests only as a count. This file is the "
        "record of which ones they are, so a switched-off test cannot pass for "
        "coverage. An ignore with a reason is a documented decision; a bare "
        "`#[ignore]` is an undocumented one, and the ratchet in the script "
        "prevents new ones."
    )
    out.append("")
    out.append(f"- **{len(entries)}** `#[ignore]` attributes across **{len({e['file'] for e in entries})}** files")
    out.append(f"- **{len(entries) - len(undocumented)}** carry a reason")
    out.append(f"- **{len(undocumented)}** do not (ratchet ceiling: {MAX_UNDOCUMENTED})")
    out.append("")

    if undocumented:
        out.append("## Undocumented — no reason recorded")
        out.append("")
        out.append("Each of these is a test switched off with nothing saying why.")
        out.append("")
        out.append("| test | location |")
        out.append("| --- | --- |")
        for e in sorted(undocumented, key=lambda x: (x["file"], x["line"])):
            out.append(f"| `{e['test']}` | `{e['file']}:{e['line']}` |")
        out.append("")

    out.append("## Documented")
    out.append("")
    for crate in sorted(by_crate):
        documented = [e for e in by_crate[crate] if e["reason"]]
        if not documented:
            continue
        out.append(f"### {crate}")
        out.append("")
        out.append("| test | reason | location |")
        out.append("| --- | --- | --- |")
        for e in sorted(documented, key=lambda x: (x["file"], x["line"])):
            reason = e["reason"].replace("|", "\\|")
            out.append(f"| `{e['test']}` | {reason} | `{e['file']}:{e['line']}` |")
        out.append("")

    return "\n".join(out) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true", help="fail if stale or over the ratchet")
    args = ap.parse_args()

    entries = collect()
    if not entries:
        # An empty scan means the crate layout moved, not that the tree is
        # clean. Refusing to write is the point: a generator that silently
        # emits nothing turns this gate into a permanent pass.
        print(f"ERROR: found no #[ignore] attributes under {CRATES} — wrong path?", file=sys.stderr)
        return 2

    rendered = render(entries)
    undocumented = sum(1 for e in entries if not e["reason"])

    if args.check:
        failures = []
        if not DOC.exists():
            failures.append(f"{DOC.relative_to(REPO)} is missing — run scripts/generate_ignored_tests.py")
        elif DOC.read_text(encoding="utf-8") != rendered:
            failures.append(
                f"{DOC.relative_to(REPO)} is stale — run scripts/generate_ignored_tests.py and commit the result"
            )
        if undocumented > MAX_UNDOCUMENTED:
            failures.append(
                f"{undocumented} bare #[ignore] attributes, ceiling is {MAX_UNDOCUMENTED}. "
                "Give the new one a reason — #[ignore = \"why\"] — rather than raising the ceiling."
            )
        if failures:
            for f in failures:
                print(f"FAIL: {f}", file=sys.stderr)
            return 1
        print(f"ok: {len(entries)} ignored tests, {undocumented} undocumented (ceiling {MAX_UNDOCUMENTED})")
        return 0

    DOC.parent.mkdir(parents=True, exist_ok=True)
    DOC.write_text(rendered, encoding="utf-8")
    print(f"wrote {DOC.relative_to(REPO)}: {len(entries)} ignored tests, {undocumented} undocumented")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
