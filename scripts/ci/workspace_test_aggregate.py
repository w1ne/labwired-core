#!/usr/bin/env python3
#
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
#
# -----------------------------------------------------------------------------
# Aggregates the pr-workspace-tests shard reports into ONE verdict.
#
# WHY A SEPARATE JOB. Each shard already fails itself on new red (fast
# feedback), but branch protection needs a single, stable context to require
# later — a matrix expands to one context per shard — and two checks need the
# FULL picture no single shard has:
#
#   1. STALE KNOWN-RED. A known_red entry whose test now PASSES must fail the
#      aggregate, so the allow-list shrinks in the PR that fixes the test
#      instead of rotting into a standing excuse that would swallow a future
#      regression of the same test. ("Shrinking it is a separate task" means a
#      separate TASK owns hunting the list down, not that the list may lie.)
#   2. MISSING REPORTS. A shard that timed out or died before uploading is
#      indistinguishable from a green shard unless someone checks that every
#      expected report exists. Absence is a failure here, never a pass.
#
# WHAT IT DISTINGUISHES. Success (the test ran and passed), known red (failed,
# and the exact package/target/test triple is allow-listed with an issue), new
# red (failed, not listed — the only thing that blocks a PR on its own), skip
# (the binary ran but printed a SKIP notice — reported in the summary, never
# silently counted as success), and hard errors (build failure / vacuous
# target, always new red).
#
# USAGE:
#   workspace_test_aggregate.py --reports <dir>
# where <dir> holds one subdirectory per uploaded shard artifact, each
# containing shard-<K>.json.
# -----------------------------------------------------------------------------

import argparse
import json
import os
import sys
from pathlib import Path

CONFIG_PATH = Path(__file__).resolve().with_name("workspace-test-shards.json")


def load_config():
    with open(CONFIG_PATH, encoding="utf-8") as fh:
        return json.load(fh)


def find_reports(reports_dir, shard_count):
    """Map shard number -> report path. Missing shards are an error, not a pass."""
    found = {}
    for path in sorted(Path(reports_dir).rglob("shard-*.json")):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            continue
        shard = data.get("shard")
        if isinstance(shard, int):
            found[shard] = path
    missing = [k for k in range(1, shard_count + 1) if k not in found]
    return found, missing


def aggregate(reports_dir, cfg):
    found, missing = find_reports(reports_dir, cfg["shard_count"])
    failures = []  # human-readable reasons the aggregate must go red
    notices = []
    totals = {"targets": 0, "passed": 0, "failed": 0, "ignored": 0, "skips": 0}
    slowest = []
    seen = {}  # (package, target, test) -> "failed" | "passed"

    for k in sorted(found):
        data = json.loads(found[k].read_text(encoding="utf-8"))
        for e in data.get("hard_errors", []):
            failures.append(
                f"shard {k}: {e['package']}/{e['target']} — {e['status']} "
                "(build failure or vacuous target; always new red)"
            )
        for e in data.get("new_red", []):
            failures.append(
                f"shard {k}: NEW RED {e['package']}/{e['target']}: {e['test']}"
            )
        for e in data.get("known_red_seen", []):
            notices.append(
                f"shard {k}: known red still red — #{e['issue']} "
                f"{e['package']}/{e['target']}: {e['test']}"
            )
        for t in data.get("targets", []):
            totals["targets"] += 1
            totals["passed"] += t.get("passed", 0)
            totals["failed"] += t.get("failed", 0)
            totals["ignored"] += t.get("ignored", 0)
            totals["skips"] += len(t.get("skips", []))
            if t["status"] == "empty":
                totals["empty"] = totals.get("empty", 0) + 1
            if t["status"] == "release-only":
                totals["release_only"] = totals.get("release_only", 0) + 1
            slowest.append(
                (t.get("duration_s", 0.0), t["package"], t["target"], t["status"])
            )
            failed_set = set(t.get("failed_tests", []))
            if t["status"] == "pass":
                # The shard ran it and nothing failed; individual test names
                # are not in the report, so record at target granularity.
                seen[(t["package"], t["target"], "*")] = "passed"
            for name in failed_set:
                seen[(t["package"], t["target"], name)] = "failed"

    for k in missing:
        failures.append(
            f"shard {k}: NO REPORT UPLOADED — the shard job timed out or died "
            "before its report existed. A missing shard is a failure, not a pass."
        )

    # Every known_red entry must be accounted for: observed failing (fine,
    # notice), or its target ran green and the entry is stale (fail), or its
    # target never ran at all (fail — the entry points at nothing).
    ran_targets = {
        (t["package"], t["target"])
        for k in found
        for t in json.loads(found[k].read_text(encoding="utf-8")).get("targets", [])
    }
    for e in cfg["known_red"]:
        key = (e["package"], e["target"], e["test"])
        if seen.get(key) == "failed":
            continue  # still red, as advertised
        if (e["package"], e["target"]) in ran_targets:
            failures.append(
                f"STALE known_red entry: #{e['issue']} {e['package']}/"
                f"{e['target']}: {e['test']} PASSED. Shrink the allow-list in "
                "this PR — remove the entry from "
                "scripts/ci/workspace-test-shards.json."
            )
        else:
            failures.append(
                f"known_red entry #{e['issue']} {e['package']}/{e['target']} "
                "did not run in any shard (renamed? deleted? newly excluded?). "
                "Fix or remove the entry."
            )

    slowest.sort(reverse=True)
    return failures, notices, totals, slowest


def main():
    ap = argparse.ArgumentParser(
        description="Aggregate the pr-workspace-tests shard reports into one verdict."
    )
    ap.add_argument("--reports", required=True)
    args = ap.parse_args()

    cfg = load_config()
    failures, notices, totals, slowest = aggregate(args.reports, cfg)

    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    lines = [
        "## Workspace tests on PR — aggregate",
        "",
        f"- targets run: {totals['targets']}",
        f"- tests passed: {totals['passed']}",
        f"- tests failed: {totals['failed']} "
        f"(known red: {len(notices)}, new red: "
        f"{sum(1 for f in failures if 'NEW RED' in f)})",
        f"- ignored: {totals['ignored']}",
        f"- lib pseudo-targets with no unit tests (heuristic overmatch, not vacuous): "
        f"{totals.get('empty', 0)}",
        f"- release-only targets (empty in debug by construction; run in the "
        f"core-integrity --release lane): {totals.get('release_only', 0)}",
        f"- skip notices printed: {totals['skips']} "
        "(a skip is not a pass; the cross-build suites are excluded from PR shards "
        "and run in core-full instead)",
        "",
        "### Slowest 15 targets",
    ]
    lines += [f"- {d:7.2f}s  {p}/{t} ({s})" for d, p, t, s in slowest[:15]]
    if notices:
        lines += ["", "### Known red (allow-listed, still failing)"]
        lines += [f"- {n}" for n in notices]
    if failures:
        lines += ["", "### Failures"]
        lines += [f"- {f}" for f in failures]
    text = "\n".join(lines) + "\n"
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write(text)
    print(text)

    if failures:
        print("AGGREGATE: RED", file=sys.stderr)
        return 1
    print("AGGREGATE: GREEN (any known red above is allow-listed with an issue)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
