#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Per-board simulator throughput gate.

WHAT IT MEASURES
    Host instructions retired per simulated CPU step ("Ir/step"), for each
    board in the matrix, running the same fixture firmware
    (`crates/firmware-perf-spin`, a bare ALU spin loop).

WHY Ir/step AND NOT WALL CLOCK
    Wall clock on a shared CI runner swings by tens of percent, which forces a
    tolerance so wide that real regressions slip through. Callgrind's retired
    instruction count is deterministic to a fraction of a percent for the same
    binary, so a 3% gate is meaningful. It is also the number that transfers:
    the browser runs the same engine through wasm, so an engine change that
    adds host work per step slows the browser by the same proportion, even
    though the absolute rate differs.

    The class of bug this exists to catch is a per-instruction cost added to a
    shared path — e.g. a `std::env::var` in `CortexM::step`, which cost ~830
    Ir/step (3x the whole engine) and was invisible to every functional test.

HOW THE FIXED COST IS REMOVED
    Each board is run twice, at two different step counts, and the per-step
    cost is the SLOPE between them. ELF loading, YAML parsing and simulator
    construction are identical in both runs, so they cancel out and never
    pollute the number.

USAGE
    python3 scripts/perf/board_perf.py                 # check against baselines
    python3 scripts/perf/board_perf.py --update        # rewrite baselines
    python3 scripts/perf/board_perf.py --boards stm32f103,stm32l476
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
BASELINE_PATH = Path(__file__).resolve().parent / "baselines.json"

FIRMWARE = REPO_ROOT / "target/thumbv6m-none-eabi/release/firmware-perf-spin"

# Every Cortex-M board whose flash/RAM layout the thumbv6m fixture links
# against (flash @ 0x08000000, RAM @ 0x20000000). Boards on other memory maps
# (nRF, RP2040, Kinetis) and other ISAs (Xtensa, RISC-V) need their own fixture
# link script and are deliberately not silently skipped — see UNCOVERED below.
BOARDS = [
    "stm32f103",
    "stm32f401",
    "stm32f401cdu6",
    "stm32f407",
    "stm32g474re",
    "stm32h563",
    "stm32h735",
    "stm32l073",
    "stm32l476",
    "stm32wb55",
    "stm32wba52",
]

# Boards the gate does NOT cover yet, and why. Printed on every run so the
# coverage hole stays visible instead of reading as "everything is measured".
UNCOVERED = {
    "nrf52840 / nrf52832 / nrf5340 / nrf54l15": "flash @ 0x00000000 — needs its own fixture link script",
    "rp2040": "flash @ 0x10000000 — needs its own fixture link script",
    "mkw41z4": "flash @ 0x00000000 — needs its own fixture link script",
    "esp32 / esp32s3 / esp32c3": "Xtensa / RISC-V — needs a per-ISA fixture",
}

STEPS_LOW = 200_000
STEPS_HIGH = 1_200_000

# Ir/step is reproducible to well under 1% for a fixed binary; 3% leaves room
# for compiler-version drift while still catching anything structural.
REGRESSION_TOLERANCE = 0.03

IREFS_RE = re.compile(r"^==\d+==\s+I\s+refs:\s+([\d,]+)", re.MULTILINE)


def measure_once(cli: Path, chip: Path, steps: int) -> int:
    """Retired host instructions for a full run of `steps` simulated steps."""
    with tempfile.TemporaryDirectory() as tmp:
        proc = subprocess.run(
            [
                "valgrind",
                "--tool=callgrind",
                f"--callgrind-out-file={tmp}/cg.out",
                "--cache-sim=no",
                "--branch-sim=no",
                str(cli),
                "run",
                "--chip",
                str(chip),
                "--firmware",
                str(FIRMWARE),
                "--max-steps",
                str(steps),
            ],
            capture_output=True,
            text=True,
        )
    match = IREFS_RE.search(proc.stderr)
    if not match:
        raise RuntimeError(
            f"callgrind produced no instruction count for {chip.name}:\n{proc.stderr[-2000:]}"
        )
    return int(match.group(1).replace(",", ""))


def measure_board(cli: Path, board: str) -> float:
    chip = REPO_ROOT / "configs/chips" / f"{board}.yaml"
    if not chip.exists():
        raise FileNotFoundError(f"no chip descriptor for board '{board}': {chip}")
    low = measure_once(cli, chip, STEPS_LOW)
    high = measure_once(cli, chip, STEPS_HIGH)
    return (high - low) / (STEPS_HIGH - STEPS_LOW)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--update",
        action="store_true",
        help="rewrite baselines.json with the measured numbers instead of gating",
    )
    parser.add_argument(
        "--boards",
        help="comma-separated subset of boards to measure (default: all)",
    )
    parser.add_argument(
        "--cli",
        default=str(REPO_ROOT / "target/release/labwired"),
        help="path to the labwired CLI built with --features event-scheduler",
    )
    args = parser.parse_args()

    if shutil.which("valgrind") is None:
        print("error: valgrind is not installed (apt-get install valgrind)", file=sys.stderr)
        return 2

    cli = Path(args.cli)
    if not cli.exists():
        print(
            f"error: CLI not found at {cli}\n"
            "  build it with: cargo build --release -p labwired-cli --features event-scheduler",
            file=sys.stderr,
        )
        return 2
    if not FIRMWARE.exists():
        print(
            f"error: fixture firmware not found at {FIRMWARE}\n"
            "  build it with: cargo build -p firmware-perf-spin --release "
            "--target thumbv6m-none-eabi",
            file=sys.stderr,
        )
        return 2

    boards = args.boards.split(",") if args.boards else BOARDS
    baselines = json.loads(BASELINE_PATH.read_text()) if BASELINE_PATH.exists() else {}

    measured: dict[str, float] = {}
    regressions: list[str] = []
    print(f"{'board':<16} {'Ir/step':>10} {'baseline':>10} {'delta':>9}")
    print("-" * 49)
    for board in boards:
        ir_per_step = measure_board(cli, board)
        measured[board] = round(ir_per_step, 1)
        base = baselines.get(board)
        if base is None:
            print(f"{board:<16} {ir_per_step:>10.1f} {'(new)':>10} {'':>9}")
            continue
        delta = (ir_per_step - base) / base
        flag = ""
        if delta > REGRESSION_TOLERANCE:
            flag = "  REGRESSION"
            regressions.append(
                f"{board}: {base:.1f} -> {ir_per_step:.1f} Ir/step ({delta:+.1%})"
            )
        elif delta < -REGRESSION_TOLERANCE:
            flag = "  (faster)"
        print(f"{board:<16} {ir_per_step:>10.1f} {base:>10.1f} {delta:>+8.1%}{flag}")

    print()
    print("not covered by this gate:")
    for group, reason in UNCOVERED.items():
        print(f"  {group}: {reason}")

    if args.update:
        merged = {**baselines, **measured}
        BASELINE_PATH.write_text(json.dumps(dict(sorted(merged.items())), indent=2) + "\n")
        print(f"\nwrote {BASELINE_PATH.relative_to(REPO_ROOT)}")
        return 0

    if regressions:
        print("\nsimulator throughput regressed:", file=sys.stderr)
        for line in regressions:
            print(f"  {line}", file=sys.stderr)
        print(
            "\nEvery extra host instruction per simulated step slows the browser "
            "twin by the same proportion.\nIf the cost is intentional, re-baseline "
            "with: python3 scripts/perf/board_perf.py --update",
            file=sys.stderr,
        )
        return 1

    print("\nno throughput regression")
    return 0


if __name__ == "__main__":
    sys.exit(main())
