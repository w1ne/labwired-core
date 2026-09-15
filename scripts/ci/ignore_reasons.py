#!/usr/bin/env python3
"""Refuse a bare `#[ignore]`: a switched-off test must say why it is switched off.

WHY THIS EXISTS

A skipped test and a passing test are indistinguishable in a green run. libtest
prints `test result: ok. 0 passed; 0 failed; 12 ignored` and exits 0, and that is
the same tick a suite that ran twelve tests earns. `scripts/ci/cargo-test-
nonvacuous.sh` catches the extreme — a target that executes NOTHING — but a
target that executes some of its tests and skips the rest is green and silent.

The only thing that separates "deliberately deferred" from "quietly abandoned"
is the reason string. This tree already has 89 excellent ones:

    #[ignore = "boots the real C3 mask ROM (~150M steps); run with --release --ignored"]
    #[ignore = "hw-oracle: requires connected NUCLEO-H563ZI"]
    #[ignore = "measurement probe, not a gate"]

Each says who can run it and what would turn it back on. A bare `#[ignore]` says
nothing, and nothing distinguishes it from a test someone gave up on.

WHAT THIS ADDS OVER `scripts/generate_ignored_tests.py`

That script inventories the same attributes into docs/testing/IGNORED_TESTS.md
and ratchets a COUNT (`MAX_UNDOCUMENTED`). A count cannot tell WHICH ones. At a
ceiling of N you may delete one bare `#[ignore]` and add a different one and the
gate stays green, because N is still N -- exactly the substitution the ledger
exists to notice. This gate is keyed on the individual tests, so an entry has to
be in the baseline by NAME to be tolerated, and a new bare ignore is rejected
even when the total does not move.

The two are complementary and both belong in CI: that one keeps the human-readable
inventory fresh, this one holds the line on the identities.

WHY THE BASELINE IS KEYED ON THE TEST, NOT ON `file:line`

`generate_ignored_tests.py` learned this the hard way: its first version recorded
`file:line`, and within the hour an unrelated pull request added 38 lines near
the top of a file, shifted seven entries down, and turned the gate red with
nothing it protects having changed. A gate that fails for a reason unrelated to
the change trains the reflex "regenerate until green".

So the key is a digest of the crate plus the test function name -- the thing you
would actually grep for, and the thing that survives a move, a reflow, or a
rename of the file around it. `file` and `line` are stored alongside as
informational fields rewritten on every `--write`, so they self-heal instead of
rotting. Renaming the FUNCTION does change the key, and that is correct: it is a
deliberate edit to the test's identity, and the fix is to write the reason rather
than to re-bless the entry.

This follows the pattern `scripts/ci/cheat_ratchet.py` established for the
CHEAT(...) surface. Same shape, same failure mode avoided.

Usage:
    ignore_reasons.py --check    # the gate
    ignore_reasons.py --write    # accept the current bare ignores as the baseline
    ignore_reasons.py --report   # list every #[ignore] and its reason
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

CORE_ROOT = Path(__file__).resolve().parents[2]
CRATES = CORE_ROOT / "crates"
BASELINE = CORE_ROOT / "validation" / "ignore_reasons_baseline.json"

# Match the attribute only at the START of a line. Several files in this tree
# explain at length, in doc comments, why something is or is not `#[ignore]`d;
# a plain `grep -c '#\[ignore\]'` counts that prose and reports 84 bare ignores
# where there are 28. Anchoring here is what makes the number reproducible.
IGNORE_START_RE = re.compile(r"^\s*#\[ignore(?P<rest>.*)$")
BARE_RE = re.compile(r"^\s*\]")
REASON_RE = re.compile(r"^\s*=\s*\"(?P<reason>(?:[^\"\\]|\\.)*)\"\s*\]")
# The `#?` accepts `fn #hw_name()` inside a proc-macro `quote!` template --
# crates/hw-oracle-macros emits its ignored hw-oracle tests that way, so there is
# no literal identifier to read. Same allowance generate_ignored_tests.py makes.
FN_RE = re.compile(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+(?P<name>#?[A-Za-z0-9_]+)")

# A reason has to say something. The shortest one in the tree when this landed
# was "diagnostic only" at 15 characters, so this floor rejects placeholders
# (`#[ignore = "x"]`, `#[ignore = "TODO"]`) without touching anything real.
MIN_REASON_CHARS = 12


def crate_of(path: Path) -> str:
    rel = path.relative_to(CRATES).parts
    return rel[0] if rel else "?"


def rust_files() -> list[Path]:
    return sorted(p for p in CRATES.rglob("*.rs") if "/target/" not in str(p))


def collect() -> list[dict]:
    """Every `#[ignore]` attribute, with the test it guards and its reason."""
    found: list[dict] = []
    for path in rust_files():
        rel = path.relative_to(CORE_ROOT).as_posix()
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        for i, line in enumerate(lines):
            m = IGNORE_START_RE.match(line)
            if not m:
                continue
            rest = m.group("rest")

            if BARE_RE.match(rest):
                reason = None
            else:
                one_line = REASON_RE.match(rest)
                if one_line:
                    reason = one_line.group("reason")
                else:
                    # A reason continued across lines with a trailing backslash.
                    # Dropping these would UNDERCOUNT the documented ones and so
                    # report real reasons as missing -- the inverse of the bug
                    # this file exists for, and just as misleading.
                    joined = rest
                    for probe in lines[i + 1 : i + 12]:
                        joined += " " + probe.strip()
                        if '"]' in probe:
                            break
                    cont = REASON_RE.match(re.sub(r"\\\s+", "", joined))
                    reason = (
                        re.sub(r"\s+", " ", cont.group("reason")).strip() if cont else None
                    )

            # The test name is the next `fn`. Attributes may sit between the
            # `#[ignore]` and the signature: #[test], #[cfg(...)], #[should_panic].
            name = "?"
            for probe in lines[i + 1 : i + 14]:
                fn = FN_RE.match(probe)
                if fn:
                    name = fn.group("name")
                    break

            found.append(
                {
                    "crate": crate_of(path),
                    "file": rel,
                    "line": i + 1,
                    "test": name,
                    "reason": reason,
                }
            )
    return found


def key_of(entry: dict) -> str:
    return hashlib.sha256(f"{entry['crate']}\n{entry['test']}".encode()).hexdigest()[:16]


def load_baseline() -> dict:
    if not BASELINE.exists():
        return {"entries": {}}
    return json.loads(BASELINE.read_text(encoding="utf-8"))


def check(entries: list[dict]) -> list[str]:
    base = load_baseline()
    known = set(base.get("entries", {}))
    errors: list[str] = []

    for e in entries:
        if e["reason"] is None:
            if key_of(e) in known:
                continue
            errors.append(
                f"{e['file']}:{e['line']}: `{e['test']}` is `#[ignore]`d with no reason.\n"
                '       Write one: #[ignore = "why, and what would turn it back on"].\n'
                "       Three already in this tree, for the three usual causes:\n"
                '         #[ignore = "hw-oracle: requires connected NUCLEO-H563ZI"]\n'
                '         #[ignore = "boots the real C3 mask ROM (~150M steps); run with '
                '--release --ignored"]\n'
                '         #[ignore = "measurement probe, not a gate"]\n'
                "       `ignore_reasons.py --write` exists for a considered exception. A\n"
                "       reason you cannot state honestly is not one: leave it bare and\n"
                "       baseline it, so the next reader still knows to ask."
            )
        elif len(e["reason"].strip()) < MIN_REASON_CHARS:
            errors.append(
                f"{e['file']}:{e['line']}: `{e['test']}` has a reason of "
                f"{len(e['reason'].strip())} characters ({e['reason'].strip()!r}), under the "
                f"{MIN_REASON_CHARS}-character floor. Say who can run it and what would turn "
                f"it back on."
            )

    # The ratchet. An entry in the baseline that now HAS a reason is progress and
    # must be dropped, or the allowance survives its own justification and the
    # slot is free for the next bare ignore to occupy silently.
    live = {key_of(e) for e in entries if e["reason"] is None}
    for stale in sorted(known - live):
        info = base["entries"][stale]
        errors.append(
            f"baseline entry `{info.get('test', stale)}` ({info.get('file', '?')}) is no longer a "
            f"bare #[ignore] -- it was documented or removed. Run "
            f"`scripts/ci/ignore_reasons.py --write` to drop it. The baseline only shrinks."
        )
    return errors


def write_baseline(entries: list[dict]) -> None:
    out = {}
    for e in sorted(entries, key=lambda x: (x["file"], x["line"])):
        if e["reason"] is not None:
            continue
        out[key_of(e)] = {
            "crate": e["crate"],
            "test": e["test"],
            # Informational, rewritten every run so a move self-heals rather than
            # rotting the way a file:line key would.
            "file": e["file"],
        }
    BASELINE.parent.mkdir(parents=True, exist_ok=True)
    BASELINE.write_text(json.dumps({"entries": out}, indent=2, sort_keys=True) + "\n")
    print(f"wrote {BASELINE.relative_to(CORE_ROOT)} — {len(out)} bare #[ignore] allowed")


def report(entries: list[dict]) -> None:
    bare = [e for e in entries if e["reason"] is None]
    print(f"{len(entries)} #[ignore] attributes across {len({e['file'] for e in entries})} files")
    print(f"{len(entries) - len(bare)} carry a reason, {len(bare)} do not")
    for e in sorted(entries, key=lambda x: (x["file"], x["line"])):
        reason = e["reason"] or "*** NO REASON ***"
        print(f"  {e['file']}:{e['line']}  {e['test']}\n      {reason}")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Refuse a bare #[ignore].")
    ap.add_argument("--check", action="store_true", help="the gate")
    ap.add_argument("--write", action="store_true", help="accept current bare ignores")
    ap.add_argument("--report", action="store_true", help="list every ignore and its reason")
    args = ap.parse_args(argv)

    entries = collect()
    if not entries:
        # An empty scan means the crate layout moved, not that the tree is clean.
        # A gate that reads zero and passes is the failure this file is about.
        print(f"ERROR: found no #[ignore] attributes under {CRATES} — wrong path?", file=sys.stderr)
        return 2

    if args.report:
        report(entries)
        return 0
    if args.write:
        write_baseline(entries)
        return 0

    errors = check(entries)
    bare = sum(1 for e in entries if e["reason"] is None)
    allowed = len(load_baseline().get("entries", {}))
    print(
        f"{len(entries)} #[ignore] attributes, {bare} bare "
        f"({allowed} allowed by validation/ignore_reasons_baseline.json)"
    )
    if errors:
        for e in errors:
            print(f"::error::{e}", file=sys.stderr)
        print(f"\n{len(errors)} problem(s).", file=sys.stderr)
        return 1
    print("every #[ignore] says why")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
