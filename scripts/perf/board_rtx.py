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
    chip_arg = board if bp.USE_CHIP_NAME else str(chip)
    env = dict(os.environ)
    start = time.perf_counter()
    proc = subprocess.run(
        [
            str(cli),
            "run",
            "--chip",
            chip_arg,
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


def fixture_could_run(chip: dict) -> bool:
    """Whether a matching perf fixture exists for this chip's memory map.

    A waiver is refused when the gate *could* measure the chip: otherwise the
    waiver would hide runnable coverage. A chip with a platform but no fixture
    (the explicit WAIVED entries for part-complete boards) is reported, not
    refused — otherwise one such waiver blocks every measured board in the run.
    """
    return bp.fixture_for(chip) is not None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--boards", help="comma-separated subset (default: all covered)")
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--seconds", type=float, default=1.0)
    parser.add_argument("--min-rtx", type=float, default=1.0)
    parser.add_argument(
        "--cli", default=str(bp.REPO_ROOT / "target/release/labwired")
    )
    parser.add_argument(
        "--chip-dir",
        default=str(bp.CHIP_DIR),
        help="directory of chip YAMLs (default: configs/chips; use a private dir for closed models)",
    )
    parser.add_argument(
        "--chip-name",
        action="store_true",
        help="pass the board name to --chip instead of the YAML path (private plugin builds)",
    )
    parser.add_argument("--status-json")
    parser.add_argument(
        "--rating-out",
        type=Path,
        help="write the RTx rating per board as JSON",
    )
    args = parser.parse_args()
    if args.repeats < 1 or args.seconds <= 0 or args.min_rtx <= 0:
        parser.error("--repeats, --seconds and --min-rtx must be positive")

    bp.CHIP_DIR = Path(args.chip_dir).resolve()
    bp.USE_CHIP_NAME = args.chip_name
    cli = Path(args.cli)
    if not cli.exists():
        print(f"error: CLI not found at {cli}", file=sys.stderr)
        return 2

    chips = bp.discover_chips()
    if args.boards:
        requested = set(args.boards.split(","))
        missing = requested - set(chips)
        if missing:
            print(f"error: unknown/uncovered boards: {', '.join(sorted(missing))}", file=sys.stderr)
            return 2
        chips = {board: chip for board, chip in chips.items() if board in requested}
    try:
        covered, waived = bp.plan_coverage(chips)
    except bp.CoverageError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    unprovable = sorted(b for b in waived if fixture_could_run(chips[b]))
    if unprovable:
        print(
            "error: RTx cannot prove all chips while waivers remain on runnable "
            "chips: " + ", ".join(unprovable),
            file=sys.stderr,
        )
        return 1
    for board, reason in sorted(waived.items()):
        print(f"not measured (no perf fixture): {board}: {reason}")

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
    if args.rating_out:
        args.rating_out.write_text(
            json.dumps(
                {
                    "schema": 1,
                    "chip_dir": str(bp.CHIP_DIR),
                    "cli": str(cli),
                    "models": {
                        board: {"rtx": row["median_rtx"], "min_rtx": row["min_rtx"]}
                        for board, row in sorted(rows.items())
                    },
                },
                indent=2,
            )
            + "\n"
        )
    if failures:
        print(
            f"\n{len(failures)} chip(s) below {args.min_rtx:.2f}x: "
            + ", ".join(failures),
            file=sys.stderr,
        )
        return 1
    unmeasured = f"; {len(waived)} waived chip(s) not measured" if waived else ""
    print(f"\nall {len(rows)} chips meet or exceed {args.min_rtx:.2f}x real time{unmeasured}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
