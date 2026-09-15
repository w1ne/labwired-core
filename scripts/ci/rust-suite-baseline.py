#!/usr/bin/env python3
"""Record how many TESTS the Rust workspace holds, per target, as a committed
baseline — and report drift against it.

Remediation row 0.1, second half. The ignored-test inventory shipped in core#966
(docs/testing/IGNORED_TESTS.md); the TypeScript half shipped as
scripts/ci/ts-suite-baseline.mjs. This is the Rust counterpart, and the only
suite baseline path that existed on main before it was scripts/perf/
baselines.json — performance numbers, not tests.

WHAT IS ALREADY COVERED, so this does not re-cover it:

  * WHICH TARGETS EXIST — scripts/ci/workspace-test-shards.json classifies every
    default-runnable test target, and workspace_test_shard.py `classify()` makes
    an unclassified target a hard error AND an exclusion entry naming no real
    target a hard error. A test binary cannot silently appear or vanish.
  * A TARGET THAT HOLDS NOTHING — crates/core/src/tests/no_vacuous_test_targets.rs
    fails an integration binary that compiles to zero tests.

WHAT WAS NOT COVERED, and is the point of this file: the number of test
FUNCTIONS inside a target that still exists and is still non-empty. A suite can
go from 40 tests to 1 — a `#[cfg(feature)]` that stops matching, a `mod tests`
that loses its `#[cfg(test)]`, a rename that orphans half a file — and every
existing gate stays green, because the target is present and non-vacuous.

DELIBERATELY NOT A GATE. Counts here move with the environment (see
preconditions below), so a hard threshold would fail for reasons that are not
regressions and would be silenced within a week. This prints drift; a human
reads it. The per-shard lane is what fails on red.

USAGE
  python3 scripts/ci/rust-suite-baseline.py            # compare to the baseline
  python3 scripts/ci/rust-suite-baseline.py --write    # re-record it
  python3 scripts/ci/rust-suite-baseline.py --json

⚠️ PRECONDITIONS — a count taken without these is not comparable:

  * FEATURE UNIFICATION. Enumeration MUST come from one
    `cargo test --workspace --no-run`, never a per-package loop. Under the
    unified build crates/wasm forces `event-scheduler` on, which both adds
    targets (`required-features` that now match) and changes runtime by 27x
    (`labwired-cli::no_elf_c3_rom_boot`: 8.59s under `-p labwired-cli`, 232s
    under the workspace build, measured 2026-08-15). This file therefore calls
    workspace_test_shard.build_workspace_tests() rather than shelling out on
    its own.
  * PROFILE. Debug. Suites whose file carries `#![cfg(not(debug_assertions))]`
    compile to an empty binary in debug — legitimately, see `is_release_only`.
    They are counted as 0 here and are NOT missing.
  * PYTHON. crates/python builds pyo3 0.20, which refuses any interpreter newer
    than 3.12. On a host with a newer python3 the workspace build FAILS — a
    toolchain gap, not red. Set PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 to build
    anyway via the stable ABI. CI's ubuntu images ship 3.12, so CI never sees
    this.
  * CROSS TARGETS. Suites that cross-build firmware at test time are excluded
    from PR shards by workspace-test-shards.json. They still BUILD here (the
    exclusion is about running), so their counts are present.
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import workspace_test_shard as shard  # noqa: E402  (path set above)

REPO_ROOT = Path(__file__).resolve().parents[2]
BASELINE = REPO_ROOT / "docs" / "testing" / "rust-suite-baseline.json"


def count_ignored(exe: str) -> int:
    """`--list --ignored` lists only the #[ignore] tests in the binary.

    Reported separately because an ignored test is present-but-not-running: a
    suite whose tests all became #[ignore] holds the same count as before and
    executes nothing.
    """
    out = subprocess.run(
        [exe, "--list", "--ignored", "--format", "terse"],
        capture_output=True, text=True, errors="replace", check=False,
    )
    return sum(1 for l in out.stdout.splitlines() if l.rstrip().endswith(": test"))


def measure() -> dict:
    targets, _ = shard.enumerate_built_targets()
    rows = []
    for t in targets:
        total = shard.count_tests(t["exe"])
        rows.append({
            "package": t["package"],
            "target": t["target"],
            "kind": t["kind"],
            "tests": total,
            "ignored": count_ignored(t["exe"]),
            "release_only": shard.is_release_only(t.get("src_path")),
        })
    rows.sort(key=lambda r: (r["package"], r["target"], r["kind"]))
    return {
        "_about": (
            "Committed test-FUNCTION counts per target. Regenerate with "
            "scripts/ci/rust-suite-baseline.py --write. Read that file's "
            "preconditions before comparing numbers taken on different hosts."
        ),
        "profile": "debug",
        "totals": {
            "targets": len(rows),
            "tests": sum(r["tests"] for r in rows),
            "ignored": sum(r["ignored"] for r in rows),
        },
        "targets": rows,
    }


def key(r: dict) -> str:
    return f"{r['package']}/{r['target']}[{r['kind']}]"


def compare(now: dict, base: dict) -> int:
    old = {key(r): r for r in base["targets"]}
    new = {key(r): r for r in now["targets"]}

    gone = sorted(set(old) - set(new))
    added = sorted(set(new) - set(old))
    shrunk = [
        (k, old[k]["tests"], new[k]["tests"])
        for k in sorted(set(old) & set(new))
        if new[k]["tests"] < old[k]["tests"]
    ]
    newly_ignored = [
        (k, old[k]["ignored"], new[k]["ignored"])
        for k in sorted(set(old) & set(new))
        if new[k]["ignored"] > old[k]["ignored"]
    ]

    ot, nt = base["totals"], now["totals"]
    print(f"baseline: {ot['targets']} targets, {ot['tests']} tests, {ot['ignored']} ignored")
    print(f"now:      {nt['targets']} targets, {nt['tests']} tests, {nt['ignored']} ignored")

    if shrunk:
        print("\nTARGETS THAT LOST TESTS — the case no existing gate catches:")
        for k, o, n in shrunk:
            print(f"  {k}: {o} -> {n}")
    if newly_ignored:
        print("\nnewly ignored (present, not running):")
        for k, o, n in newly_ignored:
            print(f"  {k}: {o} -> {n}")
    if gone:
        print("\ngone (workspace_test_shard classify() should also have caught these):")
        for k in gone:
            print(f"  {k}")
    if added:
        print(f"\nadded: {len(added)}")
        for k in added:
            print(f"  {k} ({new[k]['tests']} tests)")
    if not (shrunk or newly_ignored or gone or added):
        print("\nno drift")
    print("\nRe-record with --write once the drift above is understood.")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--write", action="store_true", help="re-record the baseline")
    ap.add_argument("--json", action="store_true", help="print measurements as JSON")
    args = ap.parse_args()

    if not os.environ.get("PYO3_USE_ABI3_FORWARD_COMPATIBILITY"):
        ver = subprocess.run(
            [sys.executable, "-c", "import sys;print('%d.%d'%sys.version_info[:2])"],
            capture_output=True, text=True,
        ).stdout.strip()
        if ver and tuple(int(x) for x in ver.split(".")) > (3, 12):
            print(
                f"note: python {ver} > pyo3 0.20's max 3.12; crates/python will fail "
                "to build. Setting PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 for this run "
                "(stable ABI). This is a host toolchain gap, not a test failure.",
                file=sys.stderr,
            )
            os.environ["PYO3_USE_ABI3_FORWARD_COMPATIBILITY"] = "1"

    now = measure()

    if args.json:
        print(json.dumps(now, indent=2))
        return 0
    if args.write:
        BASELINE.write_text(json.dumps(now, indent=2) + "\n", encoding="utf-8")
        print(f"wrote {BASELINE.relative_to(REPO_ROOT)}: "
              f"{now['totals']['targets']} targets, {now['totals']['tests']} tests")
        return 0
    if not BASELINE.exists():
        print(f"no baseline at {BASELINE.relative_to(REPO_ROOT)}; run --write",
              file=sys.stderr)
        return 2
    return compare(now, json.loads(BASELINE.read_text(encoding="utf-8")))


if __name__ == "__main__":
    sys.exit(main())
