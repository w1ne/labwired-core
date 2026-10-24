#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Measure maximum-speed real-time factor for every covered chip.

Unlike ``board_perf.py`` this is an absolute, wall-clock acceptance check.  It
runs the same compiler spin fixture through the production batched CLI path for
one simulated CPU-second, repeats the measurement, and reports the median:

    RTx = simulated CPU cycles / (wall seconds * configured CPU Hz)

Process startup is deliberately included.  That makes the number conservative
and keeps the harness honest for short browser-style runs.  The deterministic
Ir/step gate remains the sensitive regression detector; this gate answers the
separate product question: can every supported chip keep up with its clock?
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import sys
import time
from pathlib import Path

import board_perf as bp


def cpu_hz(chip: dict) -> int:
    value = chip.get("cpu_hz")
    if not isinstance(value, int) or value <= 0:
        raise ValueError(f"missing positive integer cpu_hz (got {value!r})")
    return value


def run_once(cli: Path, board: str, firmware: Path, steps: int) -> tuple[float, float]:
    """Return (wall seconds, average instructions per dispatcher batch)."""
    chip = bp.CHIP_DIR / f"{board}.yaml"
    env = dict(os.environ)
    start = time.perf_counter()
    proc = subprocess.run(
        [
            str(cli),
            "run",
            "--chip",
            str(chip),
            "--firmware",
            str(firmware),
            "--max-steps",
            str(steps),
            "--batched",
        ],
        capture_output=True,
        text=True,
        env=env,
    )
    elapsed = time.perf_counter() - start
    if proc.returncode:
        raise RuntimeError(
            f"{board}: simulator exited {proc.returncode}:\n{proc.stderr[-2000:]}"
        )
    proof = bp.BATCHED_RE.search(proc.stderr)
    if not proof:
        raise bp.ModeNotTakenError(
            f"{board}: no '[batched]' proof line; production batch path was not taken"
        )
    executed = int(proof.group(1))
    if executed != steps:
        raise bp.ModeNotTakenError(
            f"{board}: requested {steps} cycles but batched path reported {executed}"
        )
    return elapsed, float(proof.group(3))


def declared_only(chip: dict) -> bool:
    """A descriptor with no platform behind it: no peripherals, no flash, and
    no perf fixture for its memory map.

    This is the one kind of chip RTx may leave unmeasured. A waiver in
    ``board_perf.WAIVED`` wins over a matching fixture, so a waiver on its own
    could drop a chip this gate is able to run; that stays a refusal. A
    declaration with nothing to execute has no throughput to prove.
    """
    flash = chip.get("flash") or {}
    try:
        flash_size = int(flash.get("size", 0))
    except (TypeError, ValueError):
        return False
    return (
        not chip.get("peripherals")
        and flash_size == 0
        and bp.fixture_for(chip) is None
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--boards", help="comma-separated subset (default: all covered)")
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--seconds", type=float, default=1.0)
    parser.add_argument("--min-rtx", type=float, default=1.0)
    parser.add_argument(
        "--cli", default=str(bp.REPO_ROOT / "target/release/labwired")
    )
    parser.add_argument("--status-json")
    args = parser.parse_args()
    if args.repeats < 1 or args.seconds <= 0 or args.min_rtx <= 0:
        parser.error("--repeats, --seconds and --min-rtx must be positive")

    cli = Path(args.cli)
    if not cli.exists():
        print(f"error: CLI not found at {cli}", file=sys.stderr)
        return 2

    chips = bp.discover_chips()
    try:
        covered, waived = bp.plan_coverage(chips)
    except bp.CoverageError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    unprovable = sorted(b for b in waived if not declared_only(chips[b]))
    if unprovable:
        print(
            "error: RTx cannot prove all chips while waivers remain on runnable "
            "chips: " + ", ".join(unprovable),
            file=sys.stderr,
        )
        return 1
    for board, reason in sorted(waived.items()):
        print(f"not measured (declared only, no platform to run): {board}: {reason}")

    selected = set(args.boards.split(",")) if args.boards else set(covered)
    unknown = selected - set(covered)
    if unknown:
        print(f"error: unknown/uncovered boards: {', '.join(sorted(unknown))}", file=sys.stderr)
        return 2

    needed = {covered[b] for b in selected}
    try:
        firmware, skipped = bp.build_fixtures(needed)
    except (RuntimeError, subprocess.CalledProcessError) as exc:
        print(f"error: fixture build failed: {exc}", file=sys.stderr)
        return 1
    if skipped:
        print(
            "error: RTx requires every selected fixture; skipped: "
            + ", ".join(f"{name}: {why}" for name, why in sorted(skipped.items())),
            file=sys.stderr,
        )
        return 1

    rows: dict[str, dict] = {}
    failures: list[str] = []
    print("board            clock MHz   median RTx   range RTx      batch")
    print("----------------------------------------------------------------")
    for board in sorted(selected):
        hz = cpu_hz(chips[board])
        steps = max(1, round(hz * args.seconds))
        samples: list[float] = []
        widths: list[float] = []
        for _ in range(args.repeats):
            wall, width = run_once(cli, board, firmware[covered[board]], steps)
            samples.append((steps / hz) / wall)
            widths.append(width)
        median = statistics.median(samples)
        lo, hi = min(samples), max(samples)
        width = statistics.median(widths)
        rows[board] = {
            "clock_hz": hz,
            "median_rtx": median,
            "min_rtx": lo,
            "max_rtx": hi,
            "steps": steps,
            "repeats": args.repeats,
            "steps_per_batch": width,
        }
        verdict = "" if median >= args.min_rtx else "  FAIL"
        print(
            f"{board:<17} {hz / 1e6:9.1f} {median:12.2f}x "
            f"{lo:5.2f}-{hi:5.2f}x {width:10.1f}{verdict}"
        )
        if median < args.min_rtx:
            failures.append(board)

    status = {
        "ok": not failures,
        "minimum_required_rtx": args.min_rtx,
        "failures": failures,
        "measured": rows,
        "not_measured": dict(sorted(waived.items())),
    }
    if args.status_json:
        Path(args.status_json).write_text(json.dumps(status, indent=2, sort_keys=True) + "\n")
    if failures:
        print(
            f"\n{len(failures)} chip(s) below {args.min_rtx:.2f}x: "
            + ", ".join(failures),
            file=sys.stderr,
        )
        return 1
    unmeasured = f"; {len(waived)} declared-only chip(s) not measured" if waived else ""
    print(f"\nall {len(rows)} chips meet or exceed {args.min_rtx:.2f}x real time{unmeasured}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
