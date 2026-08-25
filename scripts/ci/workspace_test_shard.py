#!/usr/bin/env python3
#
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
#
# -----------------------------------------------------------------------------
# Runs one shard of `cargo test --workspace` on a pull request.
#
# WHY THIS EXISTS. `cargo test --workspace` — 270+ test binaries across 79
# packages — used to execute for the first time the night AFTER a merge, in
# the `core-full` job. This script is the PR-lane half of the fix
# (remediation plan task 2.3): it splits that run into N deterministic shards
# (the workflow runs them as a matrix) and reports structured results an
# aggregate job renders a verdict from.
#
# WHY IT BUILDS ONCE AND RUNS BINARIES DIRECTLY. `cargo test --workspace` is
# NOT the same thing as `cargo test -p <pkg>` repeated per package: cargo's
# feature unification switches `event-scheduler` ON for labwired-core whenever
# the whole workspace is built, because crates/wasm requests it — so in the
# nightly, every labwired-core integration test, every labwired-cli test and
# the hardware oracle all run against a scheduler-enabled core, and the
# `required-features = ["event-scheduler"]` targets run too. Measured proof
# that the difference is real: `environment_runner_prioritizes_failed_
# assertions_over_runtime_errors` (core#929) PASSES under `-p labwired-cli`
# and FAILS under `--workspace`. A per-package shard loop would silently test
# a different program than the nightly does.
#
# So the flow is the same one cargo-test-nonvacuous.sh uses: build the whole
# workspace's test set in ONE invocation (`--no-run --message-format=json`,
# identical unification to the nightly), then execute each test binary
# straight out of the artifact list, with the package's manifest dir as cwd —
# the same cwd `cargo test` would give it. Doctests are not built by
# `--no-run`, so they stay nightly-only; an empty doctest target legitimately
# reports `0 passed`, which is the false-green shape this repo guards against.
#
# ANTI-VACUITY. Every binary is asked how many tests it holds (`--list`,
# cargo-test-nonvacuous.sh's exact mechanism). An INTEGRATION binary listing
# zero is a hard failure — an empty suite and a passing suite print the same
# colour. A unittest (lib/bin) binary listing zero is reported as `empty`,
# not red: those come from a src/-grep-level classification where `#[test]`
# also matches proc-macro template strings, and "crate with no unit tests" is
# a legitimate shape. The bug class that matters — an integration FILE
# emptied by a `#[cfg]` — only exists in integration binaries.
#
# EXCLUDED, BY DESIGN: the cross-build suites in workspace-test-shards.json
# (they build firmware for thumbv*/xtensa at RUN time, or panic without a
# pre-built cross ELF; core-full installs the five rustup targets and runs
# them nightly, so PR shards lose no coverage by skipping them). The
# classification is closed: a built target in NEITHER the shard run nor the
# exclusion list is a hard error, so a new test file cannot silently miss the
# lane, and an exclusion or known_red entry naming a target that no longer
# builds is an error too — the lists cannot rot into excuses.
#
# KNOWN-RED. Test failures matching known_red (exact package/target/test-name
# triples, one GitHub issue each — no patterns, so an unrelated failure in the
# same file can never hide behind an entry) are reported but do not fail the
# shard. Any OTHER failure does. The aggregate job re-derives the same verdict
# from the uploaded reports and additionally fails when an allow-listed test
# PASSES, so the list shrinks in the PR that fixes the test instead of
# silently re-arming.
#
# USAGE:
#   workspace_test_shard.py plan                       # approximate split (no build)
#   workspace_test_shard.py run --shard K --report F   # build + execute shard K
# -----------------------------------------------------------------------------

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
CONFIG_PATH = Path(__file__).resolve().with_name("workspace-test-shards.json")

# Matches libtest's per-binary summary:
#   test result: ok. 24 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.56s
RESULT_RE = re.compile(
    r"test result: (ok|FAILED)\.\s+(\d+) passed; (\d+) failed; (\d+) ignored; "
    r"(\d+) measured; (\d+) filtered out(?:; finished in ([0-9.]+)s)?"
)
# `test some::name ... FAILED` — the reliable source of failed test names.
FAILED_TEST_RE = re.compile(r"^test (\S+) \.\.\. FAILED\s*$", re.MULTILINE)
# Fallback for panic=abort kills, where libtest may not print the FAILED line:
#   thread 'some::name' (12345) panicked at ...
PANIC_RE = re.compile(r"^thread '([^']+)' \(\d+\) panicked", re.MULTILINE)
# The skip channel: test_support::skip_or_fail_missing_firmware prints
# `SKIP: <what> not built; ...` to stderr; a few older files print `[skip]`.
SKIP_RE = re.compile(r"SKIP: |\[skip\]|\[SKIP\]")


def load_config():
    with open(CONFIG_PATH, encoding="utf-8") as fh:
        return json.load(fh)


def workspace_metadata():
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version=1"],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
        errors="replace",
    )
    meta = json.loads(out.stdout)
    by_id = {}
    for pkg in meta["packages"]:
        by_id[pkg["id"]] = {
            "name": pkg["name"],
            "manifest_dir": str(Path(pkg["manifest_path"]).parent),
        }
    return meta, by_id


def build_workspace_tests():
    """`cargo test --workspace --no-run`, returning the artifact list cargo
    itself produced. Feature unification here is identical to the nightly's
    `cargo test --workspace` by construction. A build failure is always new
    red — a test target that does not COMPILE is the class crates/core's
    release-only and hw-oracle rot taught — so a nonzero exit raises."""
    proc = subprocess.run(
        ["cargo", "test", "--workspace", "--no-run", "--message-format=json"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        errors="replace",
        check=False,
    )
    if proc.returncode != 0:
        tail = "\n".join((proc.stdout + proc.stderr).splitlines()[-80:])
        raise RuntimeError(
            "cargo test --workspace --no-run FAILED (exit "
            f"{proc.returncode}). A test target that does not compile is new "
            f"red, full stop. Output tail:\n{tail}"
        )
    artifacts = []
    bin_exes = {}
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") != "compiler-artifact":
            continue
        if not msg.get("executable"):
            continue
        if msg.get("profile", {}).get("test"):
            artifacts.append(msg)
        elif "bin" in msg.get("target", {}).get("kind", []):
            # The plain (non-test-profile) binaries, for CARGO_BIN_EXE_* —
            # cargo sets one of these per same-package bin when it launches
            # an integration test.
            bin_exes.setdefault(msg["package_id"], []).append(
                (msg["target"]["name"], msg["executable"])
            )
    return artifacts, bin_exes


def enumerate_built_targets():
    """(targets, bin_exes): one entry per test binary the unified workspace
    build produced. kind is 'test' for integration binaries and 'lib'/'bin'
    for unittest binaries (both hold #[cfg(test)] unit tests). bin_exes maps
    package_id -> [(bin name, executable)] for the CARGO_BIN_EXE_* env."""
    _, by_id = workspace_metadata()
    out = []
    artifacts, bin_exes = build_workspace_tests()
    for msg in artifacts:
        pkg = by_id.get(msg["package_id"])
        if pkg is None:
            continue
        t = msg["target"]
        # Unittest binaries: a package's lib AND each of its bins gets one
        # (several bins can share the package's name, so package+target does
        # not dedupe them — that is correct, cargo test runs them all).
        if "test" in t["kind"]:
            kind = "test"
        elif "bin" in t["kind"]:
            kind = "bin"
        else:
            kind = "lib"
        out.append(
            {
                "package": pkg["name"],
                "package_id": msg["package_id"],
                "target": t["name"],
                "kind": kind,
                "exe": msg["executable"],
                "cwd": pkg["manifest_dir"],
                "src_path": t.get("src_path"),
            }
        )
    out.sort(key=lambda e: (e["package"], e["target"], e["kind"]))
    return out, bin_exes


def test_env(entry, bin_exes):
    """The environment cargo test would have given this binary. Running the
    executables directly is only faithful if these match:

    - CARGO_MANIFEST_DIR — the package root; tests read fixtures and configs
      relative to it at RUN time (cli_integration.rs unwraps it).
    - CARGO_BIN_EXE_<name> — one per bin of the SAME package, pointing at the
      plain (non-test-profile) binary cargo just built.
    - CARGO_TARGET_TMPDIR — <target>/tmp; test_support::target_dir() resolves
      the build tree from its parent, and some suites write scratch files
      there.
    """
    env = {
        "CARGO_MANIFEST_DIR": entry["cwd"],
    }
    deps = Path(entry["exe"]).parent
    if deps.name == "deps":
        target = deps.parent.parent
        tmp = target / "tmp"
        tmp.mkdir(parents=True, exist_ok=True)
        env["CARGO_TARGET_TMPDIR"] = str(tmp)
    for name, exe in bin_exes.get(entry["package_id"], []):
        env[f"CARGO_BIN_EXE_{name.replace('-', '_')}"] = exe
    return env


def enumerate_targets_approx():
    """Build-free approximation for `plan` (humans and dry-runs): metadata
    only. UNDER-COUNTS on purpose and says so — targets whose
    `required-features` are satisfied by workspace feature unification
    (event-scheduler, via crates/wasm) only appear after a real build. `run`
    is authoritative."""
    meta, _ = workspace_metadata()
    targets = []
    for pkg in meta["packages"]:
        name = pkg["name"]
        for t in pkg["targets"]:
            if "test" in t["kind"]:
                targets.append(
                    {"package": name, "target": t["name"], "kind": "test",
                     "exe": None, "cwd": str(Path(pkg["manifest_path"]).parent),
                     "src_path": t.get("src_path"),
                     # Gated targets are enumerated (so exclusion entries can be
                     # validated against them) but not assigned to shards: the
                     # approximation cannot model feature unification.
                     "gated": bool(t.get("required-features"))}
                )
        src = Path(pkg["manifest_path"]).parent / "src"
        if src.is_dir() and any(
            b"#[test]" in Path(p).read_bytes() for p in src.rglob("*.rs")
        ):
            targets.append(
                {"package": name, "target": name, "kind": "lib",
                 "exe": None, "cwd": str(Path(pkg["manifest_path"]).parent),
                 "src_path": None, "gated": False}
            )
    targets.sort(key=lambda e: (e["package"], e["target"], e["kind"]))
    return targets


def classify(cfg, targets):
    """Validate the config against the enumerated target set. Returns
    (runnable, problems): every exclusion and known_red entry must name a real
    target, or the lists rot into excuses."""
    have = {(t["package"], t["target"]) for t in targets if t["kind"] == "test"}
    excluded = {(e["package"], e["target"]) for e in cfg["cross_build_excluded"]}
    excluded |= {
        (e["package"], e["target"]) for e in cfg.get("nightly_only_excluded", [])
    }
    # Cost-only exclusions, kept in their own list because they are revoked for
    # a different reason than the scheduler ones — see `_pr_cost_about`.
    excluded |= {
        (e["package"], e["target"]) for e in cfg.get("pr_cost_excluded", [])
    }
    known = {(e["package"], e["target"]) for e in cfg["known_red"]}
    problems = []
    for p, t in sorted(excluded - have):
        problems.append(
            f"exclusion entry {p}/{t} names no built test target "
            "(renamed? deleted? feature-gated now?). Remove or fix the entry."
        )
    for p, t in sorted(known - have):
        problems.append(
            f"known_red entry for {p}/{t} names no built test target. If the "
            "test moved or was fixed and deleted, shrink the allow-list."
        )
    runnable = [
        t for t in targets
        if not t.get("gated") and (t["package"], t["target"]) not in excluded
    ]
    return runnable, problems


def shard_slice(runnable, shard, shard_count):
    """Deterministic round-robin over the sorted target list. Round-robin (not
    contiguous blocks) so that one package's heavy targets — labwired-core has
    ~125 integration binaries — spread across all shards instead of landing in
    one."""
    return [entry for i, entry in enumerate(runnable) if i % shard_count == shard - 1]


def count_tests(exe):
    listing = subprocess.run(
        [exe, "--list", "--format", "terse"],
        capture_output=True, text=True, errors="replace", check=False,
    )
    return sum(1 for l in listing.stdout.splitlines() if l.rstrip().endswith(": test"))


def is_release_only(src_path):
    """True when the integration file's inner `#![cfg(...)]` ALSO carries
    `not(debug_assertions)` — release-only by construction, so a debug build
    legitimately compiles it to an empty binary (the RELEASE_ONLY category in
    crates/core/src/tests/no_vacuous_test_targets.rs). `required-features`
    covers the feature half of that gate and cannot cover the other half."""
    if not src_path:
        return False
    try:
        src = Path(src_path).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return False
    return any(
        line.strip().startswith("#![cfg(") and "not(debug_assertions)" in line
        for line in src.splitlines()
    )


def parse_run(output):
    """Parse one test binary's combined output into a dict."""
    failed_tests = sorted(
        set(FAILED_TEST_RE.findall(output)) | set(PANIC_RE.findall(output))
    )
    skips = sorted(
        {line.strip() for line in output.splitlines() if SKIP_RE.search(line)}
    )
    results = RESULT_RE.findall(output)
    return {
        "passed": sum(int(r[1]) for r in results),
        "failed": sum(int(r[2]) for r in results),
        "ignored": sum(int(r[3]) for r in results),
        "filtered": sum(int(r[5]) for r in results),
        "duration_s": round(sum(float(r[6]) for r in results if r[6]), 2),
        "result_lines": len(results),
        "failed_tests": failed_tests,
        "skips": skips,
    }


def run_one(entry, listed, extra_env):
    proc = subprocess.run(
        [entry["exe"]],
        cwd=entry["cwd"],
        env={**os.environ, **extra_env},
        capture_output=True,
        text=True,
        # Test binaries are not guaranteed to print UTF-8 (stress/ratchet
        # suites dump raw bytes); a decode crash is not a test verdict.
        errors="replace",
        # The suite is allowed to fail; the caller classifies the failures.
        check=False,
    )
    output = proc.stdout + proc.stderr
    parsed = parse_run(output)
    status = "pass"
    if parsed["result_lines"] == 0:
        # No libtest summary at all: the binary died before reporting (abort,
        # harness crash). Never attributable to a known-red test.
        status = "error"
    elif (
        parsed["passed"] == 0
        and parsed["failed"] == 0
        and parsed["ignored"] == 0
        and parsed["filtered"] == 0
    ):
        # Listed tests but ran none: every filter/cfg path emptied the run.
        status = "vacuous"
    elif parsed["failed"] > 0 or proc.returncode != 0:
        status = "fail"
    return {
        "package": entry["package"],
        "target": entry["target"],
        "kind": entry["kind"],
        "status": status,
        "listed": listed,
        "exit_code": proc.returncode,
        **parsed,
        "log": output,
    }


def cmd_plan(args):
    cfg = load_config()
    targets = enumerate_targets_approx()
    runnable, problems = classify(cfg, targets)
    shard_count = args.shards or cfg["shard_count"]
    for p in problems:
        print(f"ERROR: {p}", file=sys.stderr)
    if problems:
        return 2
    print(
        f"{len(targets)} targets (APPROXIMATE — pre-build metadata; the real "
        f"run also picks up targets whose required-features feature "
        f"unification enables, e.g. event-scheduler); "
        f"{len(cfg['cross_build_excluded'])} cross-build-excluded, "
        f"{len(cfg.get('nightly_only_excluded', []))} nightly-only-excluded, "
        f"{len(cfg.get('pr_cost_excluded', []))} cost-excluded "
        f"({sum(e.get('seconds', 0) for e in cfg.get('pr_cost_excluded', [])):.0f}s of measured PR time); "
        f"{len(runnable)} in the PR shards across {shard_count} shard(s)"
    )
    for k in range(1, shard_count + 1):
        slice_ = shard_slice(runnable, k, shard_count)
        print(f"\n== shard {k}/{shard_count}: {len(slice_)} targets ==")
        by_pkg = {}
        for e in slice_:
            by_pkg.setdefault(e["package"], []).append(e)
        for p in sorted(by_pkg):
            names = [
                "--lib" if e["kind"] == "lib" else f"--test {e['target']}"
                for e in by_pkg[p]
            ]
            print(f"  {p}: " + " ".join(names))
    return 0


def cmd_run(args):
    cfg = load_config()
    shard_count = cfg["shard_count"]
    try:
        targets, bin_exes = enumerate_built_targets()
    except RuntimeError as e:
        print(str(e), file=sys.stderr)
        # No report could exist that explains this as a test outcome; write a
        # minimal one anyway so the aggregate can attribute the red.
        Path(args.report).write_text(
            json.dumps(
                {
                    "shard": args.shard,
                    "shard_count": shard_count,
                    "targets": [],
                    "hard_errors": [
                        {"package": "<workspace>", "target": "<build>",
                         "status": "error"}
                    ],
                    "new_red": [],
                    "known_red_seen": [],
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        return 1
    runnable, problems = classify(cfg, targets)
    if problems:
        for p in problems:
            print(f"ERROR: {p}", file=sys.stderr)
        return 2
    slice_ = shard_slice(runnable, args.shard, shard_count)
    print(f"{len(targets)} test binaries built; shard {args.shard}/{shard_count}: "
          f"{len(slice_)} targets")

    known_red = {
        (e["package"], e["target"], e["test"]): e["issue"] for e in cfg["known_red"]
    }
    results = []
    hard_errors = []
    new_red = []
    known_hits = []
    for i, entry in enumerate(slice_, 1):
        label = (
            f"{entry['package']}::{entry['target']}"
            if entry["kind"] == "test"
            else f"{entry['package']}::{entry['target']} ({entry['kind']} unittests)"
        )
        print(f"[{i}/{len(slice_)}] {label}", flush=True)
        listed = count_tests(entry["exe"])
        if listed == 0 and entry["kind"] in ("lib", "bin"):
            r = {
                "package": entry["package"], "target": entry["target"],
                "kind": entry["kind"], "status": "empty", "listed": 0, "exit_code": 0,
                "passed": 0, "failed": 0, "ignored": 0, "filtered": 0,
                "duration_s": 0.0, "result_lines": 0,
                "failed_tests": [], "skips": [],
                "log": "unittest binary lists 0 tests (no #[cfg(test)] unit tests)",
            }
            results.append(r)
            print("    empty: no unit tests in this binary", flush=True)
            continue
        if listed == 0 and is_release_only(entry.get("src_path")):
            r = {
                "package": entry["package"], "target": entry["target"],
                "kind": entry["kind"], "status": "release-only", "listed": 0,
                "exit_code": 0, "passed": 0, "failed": 0, "ignored": 0,
                "filtered": 0, "duration_s": 0.0, "result_lines": 0,
                "failed_tests": [], "skips": [],
                "log": "empty IN DEBUG by construction: the file's inner cfg "
                       "carries not(debug_assertions); it runs in the "
                       "core-integrity --release lane, not here.",
            }
            results.append(r)
            print("    release-only: empty in debug by construction", flush=True)
            continue
        if listed == 0:
            r = {
                "package": entry["package"], "target": entry["target"],
                "kind": entry["kind"], "status": "vacuous", "listed": 0,
                "exit_code": 0, "passed": 0, "failed": 0, "ignored": 0,
                "filtered": 0, "duration_s": 0.0, "result_lines": 0,
                "failed_tests": [], "skips": [],
                "log": "integration binary lists ZERO tests — the vacuous-green "
                       "shape cargo-test-nonvacuous.sh exists to catch",
            }
            results.append(r)
            hard_errors.append(r)
            print("    VACUOUS: integration binary lists 0 tests", flush=True)
            continue
        r = run_one(entry, listed, test_env(entry, bin_exes))
        results.append(r)
        print(
            f"    {r['status']}: {r['passed']} passed, {r['failed']} failed, "
            f"{r['ignored']} ignored in {r['duration_s']}s",
            flush=True,
        )
        if r["status"] in ("error", "vacuous"):
            hard_errors.append(r)
            continue
        for t in r["failed_tests"]:
            key = (entry["package"], entry["target"], t)
            if key in known_red:
                known_hits.append((key, known_red[key]))
            else:
                new_red.append(key)
        if r["status"] == "fail" and not r["failed_tests"]:
            # A nonzero exit with no parseable test name is not attributable —
            # treat it as new red at target granularity rather than dropping it.
            new_red.append((entry["package"], entry["target"], "<unattributed failure>"))

    log_dir = Path(args.report).parent / (Path(args.report).stem + "-logs")
    log_dir.mkdir(parents=True, exist_ok=True)
    slim = []
    for r in results:
        log_name = f"{r['package']}--{r['target']}.log"
        (log_dir / log_name).write_text(r.pop("log"), encoding="utf-8")
        r["log_file"] = str(log_dir / log_name)
        slim.append(r)
    report = {
        "shard": args.shard,
        "shard_count": shard_count,
        "built_targets": len(targets),
        "targets": slim,
        "hard_errors": [
            {"package": r["package"], "target": r["target"], "status": r["status"]}
            for r in hard_errors
        ],
        "new_red": [
            {"package": p, "target": t, "test": n} for p, t, n in new_red
        ],
        "known_red_seen": [
            {"package": p, "target": t, "test": n, "issue": i}
            for (p, t, n), i in known_hits
        ],
    }
    Path(args.report).write_text(json.dumps(report, indent=2), encoding="utf-8")

    total_pass = sum(r["passed"] for r in results)
    total_ign = sum(r["ignored"] for r in results)
    print(f"\nshard {args.shard} done: {total_pass} passed, {total_ign} ignored, "
          f"{len(new_red)} new-red, {len(known_hits)} known-red, "
          f"{len(hard_errors)} hard error(s)")

    if hard_errors:
        print("\nHARD ERRORS (build failure or vacuous target — always new red):",
              file=sys.stderr)
        for r in hard_errors:
            print(f"  {r['package']}/{r['target']}: {r['status']} "
                  f"(see {r['log_file']})", file=sys.stderr)
    if new_red:
        print("\nNEW RED (not in the known-red allow-list):", file=sys.stderr)
        for p, t, n in new_red:
            print(f"  {p}/{t}: {n}", file=sys.stderr)
        print("Fix the test, or — only if it is red on main for a tracked reason — "
              "add it to known_red in scripts/ci/workspace-test-shards.json with an issue number.",
              file=sys.stderr)
    if known_hits:
        print("\nknown-red still red (allowed):")
        for (p, t, n), i in sorted(set(known_hits)):
            print(f"  #{i}: {p}/{t}: {n}")
    return 1 if (hard_errors or new_red) else 0


def main():
    ap = argparse.ArgumentParser(
        description="Run one shard of 'cargo test --workspace' on a pull request."
    )
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("plan", help="print the approximate shard plan, run nothing")
    p.add_argument("--shards", type=int, default=None)
    p.set_defaults(fn=cmd_plan)
    r = sub.add_parser("run", help="build the workspace test set and run one shard")
    r.add_argument("--shard", type=int, required=True)
    r.add_argument("--report", required=True)
    r.set_defaults(fn=cmd_run)
    args = ap.parse_args()
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
