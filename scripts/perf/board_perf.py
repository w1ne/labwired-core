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

WHICH BOARDS ARE COVERED
    Every chip descriptor in `configs/chips/` is measured unless it is WAIVED
    below with a reason. Coverage is *derived* from the descriptors, not from a
    hand-kept list, because a hand-kept list silently stops covering whatever
    is added after it was last edited — which is how stm32f405, stm32f411ceu6,
    stm32f767 and rp2350 ended up outside the gate without appearing in its
    "not covered" note. A chip that is neither measurable nor waived is a hard
    error, so adding a chip forces a decision either way.

WHY A BASELINE THAT IS TOO HIGH ALSO FAILS
    A board that measures far *below* its baseline is not good news, it is a
    dead gate: the slack is exactly how much it can regress before anyone is
    told. Improvements have to be locked in with --update, same as accepted
    costs.

USAGE
    python3 scripts/perf/board_perf.py                 # check against baselines
    python3 scripts/perf/board_perf.py --update        # rewrite baselines
    python3 scripts/perf/board_perf.py --boards stm32f103,stm32l476
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
BASELINE_PATH = Path(__file__).resolve().parent / "baselines.json"
CHIP_DIR = REPO_ROOT / "configs/chips"

FIXTURE_TARGET = "thumbv6m-none-eabi"
FIXTURE_DIR = REPO_ROOT / "target/perf-fixtures"

# One linked image per memory map. The spin loop is identical in all of them —
# thumbv6m runs unchanged on M0+ through M33 — so a board's number stays
# comparable to every other board's; only the link origins differ.
#
# Keyed by (flash base, RAM base) as read from the chip descriptor, so a chip
# is matched to a fixture by what it actually models rather than by name.
FIXTURES = {
    (0x08000000, 0x20000000): "stm32",  # STM32 family
    (0x00000000, 0x20000000): "nrf",  # Nordic nRF52/nRF53/nRF54
    (0x00000000, 0x1FFF8000): "kinetis",  # NXP Kinetis (MKW41Z4)
    (0x10000000, 0x20000000): "rp2xxx",  # Raspberry Pi RP2040 / RP2350
}

# Chips the gate cannot measure, with the reason. Anything here is reported on
# every run; anything neither here nor matched to a fixture aborts the run.
WAIVED = {
    "esp32": "Xtensa — needs the esp-rs rustc fork, not available on the CI image",
    "esp32s3": "Xtensa — needs the esp-rs rustc fork, not available on the CI image",
    "esp32s3-zero": "Xtensa — needs the esp-rs rustc fork, not available on the CI image",
    "esp32c3": "RISC-V — needs a riscv32imc fixture crate (cortex-m-rt cannot link it)",
}

# Descriptors that are CI plumbing rather than a modelled part.
CHIP_EXCLUDE_PREFIX = "ci-fixture-"

STEPS_LOW = 200_000
STEPS_HIGH = 1_200_000

# Ir/step is reproducible to well under 1% for a fixed binary; 3% leaves room
# for compiler-version drift while still catching anything structural.
REGRESSION_TOLERANCE = 0.03

# How far a baseline may sit above the measured cost before it counts as stale.
# Wider than the regression tolerance so an ordinary optimisation does not trip
# the gate the moment it lands, narrow enough that a 2x-slack baseline cannot
# sit there for months hiding real regressions underneath it.
STALE_TOLERANCE = 0.10

IREFS_RE = re.compile(r"^==\d+==\s+I\s+refs:\s+([\d,]+)", re.MULTILINE)


class CoverageError(RuntimeError):
    """A chip is neither measurable nor explicitly waived."""


def discover_chips() -> dict[str, dict]:
    """Every modelled chip descriptor, by board id (the file stem)."""
    chips: dict[str, dict] = {}
    for path in sorted(CHIP_DIR.glob("*.yaml")):
        if path.stem.startswith(CHIP_EXCLUDE_PREFIX):
            continue
        chips[path.stem] = yaml.safe_load(path.read_text()) or {}
    return chips


def fixture_for(chip: dict) -> str | None:
    """Which linked fixture a chip's memory map needs, if any covers it."""
    if chip.get("arch") != "arm":
        return None
    try:
        key = (int(chip["flash"]["base"]), int(chip["ram"]["base"]))
    except (KeyError, TypeError, ValueError):
        return None
    return FIXTURES.get(key)


def plan_coverage(chips: dict[str, dict]) -> tuple[dict[str, str], dict[str, str]]:
    """Split the chip set into {board: fixture} and {board: waiver reason}.

    Raises CoverageError for any chip that is neither, so a newly added part
    cannot quietly drop out of the gate.
    """
    covered: dict[str, str] = {}
    waived: dict[str, str] = {}
    unclassified: list[str] = []
    for board, chip in chips.items():
        fixture = fixture_for(chip)
        if fixture is not None:
            covered[board] = fixture
        elif board in WAIVED:
            waived[board] = WAIVED[board]
        else:
            flash = chip.get("flash", {}).get("base")
            ram = chip.get("ram", {}).get("base")
            arch = chip.get("arch", "?")
            where = (
                f"arch={arch} flash={flash:#010x} ram={ram:#010x}"
                if isinstance(flash, int) and isinstance(ram, int)
                else f"arch={arch} flash={flash} ram={ram}"
            )
            unclassified.append(f"{board} ({where})")
    if unclassified:
        raise CoverageError(
            "these chips are neither covered by a perf fixture nor waived:\n  "
            + "\n  ".join(sorted(unclassified))
            + "\n\nAdd the memory map to FIXTURES (and build the fixture in "
            "core-perf.yml), or add an entry to WAIVED with the reason.\n"
            "Leaving a chip unclassified is not an option: the gate would "
            "report 'covered' while measuring nothing for it."
        )
    return covered, waived


def build_fixtures(fixtures: set[str]) -> dict[str, Path]:
    """Link the spin loop once per memory map; return {fixture: ELF path}."""
    origins = {name: key for key, name in FIXTURES.items()}
    FIXTURE_DIR.mkdir(parents=True, exist_ok=True)
    built: dict[str, Path] = {}
    for name in sorted(fixtures):
        flash, ram = origins[name]
        env_note = f"flash={flash:#010x} ram={ram:#010x}"
        out = FIXTURE_DIR / f"firmware-perf-spin-{name}"
        print(f"building fixture '{name}' ({env_note})", file=sys.stderr)
        subprocess.run(
            [
                "cargo",
                "build",
                "-p",
                "firmware-perf-spin",
                "--release",
                "--target",
                FIXTURE_TARGET,
            ],
            cwd=REPO_ROOT,
            check=True,
            env={
                **os.environ,
                "LABWIRED_PERF_FLASH_ORIGIN": f"{flash:#010x}",
                "LABWIRED_PERF_RAM_ORIGIN": f"{ram:#010x}",
            },
        )
        src = REPO_ROOT / f"target/{FIXTURE_TARGET}/release/firmware-perf-spin"
        shutil.copy2(src, out)
        built[name] = out
    return built


def measure_once(cli: Path, chip: Path, firmware: Path, steps: int) -> int:
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
                str(firmware),
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


def measure_board(cli: Path, board: str, firmware: Path) -> float:
    chip = CHIP_DIR / f"{board}.yaml"
    if not chip.exists():
        raise FileNotFoundError(f"no chip descriptor for board '{board}': {chip}")
    low = measure_once(cli, chip, firmware, STEPS_LOW)
    high = measure_once(cli, chip, firmware, STEPS_HIGH)
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
        help="comma-separated subset of boards to measure (default: all covered)",
    )
    parser.add_argument(
        "--cli",
        default=str(REPO_ROOT / "target/release/labwired"),
        help="path to the labwired CLI built with --features event-scheduler",
    )
    parser.add_argument(
        "--status-json",
        help="write a machine-readable result here (used by the CI issue step)",
    )
    parser.add_argument(
        "--check-coverage",
        action="store_true",
        help="only verify every chip is covered or waived, then exit (no build, "
        "no valgrind) — cheap enough to run on every PR",
    )
    args = parser.parse_args()

    if args.check_coverage:
        try:
            covered, waived = plan_coverage(discover_chips())
        except CoverageError as exc:
            print(f"error: {exc}", file=sys.stderr)
            return 1
        print(
            f"perf gate covers {len(covered)} chips across "
            f"{len(set(covered.values()))} memory maps; {len(waived)} waived"
        )
        for board, reason in sorted(waived.items()):
            print(f"  waived: {board}: {reason}")
        return 0

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

    try:
        covered, waived = plan_coverage(discover_chips())
    except CoverageError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    boards = args.boards.split(",") if args.boards else sorted(covered)
    unknown = [b for b in boards if b not in covered]
    if unknown:
        print(
            f"error: not covered by any fixture: {', '.join(unknown)}",
            file=sys.stderr,
        )
        return 2

    firmware = build_fixtures({covered[b] for b in boards})
    baselines = json.loads(BASELINE_PATH.read_text()) if BASELINE_PATH.exists() else {}

    measured: dict[str, float] = {}
    regressions: list[dict] = []
    stale: list[dict] = []
    print(f"{'board':<16} {'fixture':<9} {'Ir/step':>10} {'baseline':>10} {'delta':>9}")
    print("-" * 59)
    for board in boards:
        fixture = covered[board]
        ir_per_step = measure_board(cli, board, firmware[fixture])
        measured[board] = round(ir_per_step, 1)
        base = baselines.get(board)
        if base is None:
            print(f"{board:<16} {fixture:<9} {ir_per_step:>10.1f} {'(new)':>10} {'':>9}")
            continue
        delta = (ir_per_step - base) / base
        entry = {
            "board": board,
            "baseline": round(base, 1),
            "measured": round(ir_per_step, 1),
            "delta": round(delta, 4),
        }
        flag = ""
        if delta > REGRESSION_TOLERANCE:
            flag = "  REGRESSION"
            regressions.append(entry)
        elif delta < -STALE_TOLERANCE:
            flag = "  STALE BASELINE"
            stale.append(entry)
        elif delta < -REGRESSION_TOLERANCE:
            flag = "  (faster)"
        print(
            f"{board:<16} {fixture:<9} {ir_per_step:>10.1f} {base:>10.1f} "
            f"{delta:>+8.1%}{flag}"
        )

    print()
    print(f"covered: {len(covered)} chips across {len(set(covered.values()))} memory maps")
    print("not covered by this gate:")
    for board, reason in sorted(waived.items()):
        print(f"  {board}: {reason}")

    ok = not regressions and not stale
    if args.status_json:
        Path(args.status_json).write_text(
            json.dumps(
                {
                    "ok": ok,
                    "regressions": regressions,
                    "stale": stale,
                    "covered": {b: covered[b] for b in boards},
                    "waived": waived,
                    "measured": measured,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n"
        )

    if args.update:
        merged = {**baselines, **measured}
        BASELINE_PATH.write_text(json.dumps(dict(sorted(merged.items())), indent=2) + "\n")
        print(f"\nwrote {BASELINE_PATH.relative_to(REPO_ROOT)}")
        return 0

    if regressions:
        print("\nsimulator throughput regressed:", file=sys.stderr)
        for entry in regressions:
            print(
                f"  {entry['board']}: {entry['baseline']:.1f} -> {entry['measured']:.1f} "
                f"Ir/step ({entry['delta']:+.1%})",
                file=sys.stderr,
            )
        print(
            "\nEvery extra host instruction per simulated step slows the browser "
            "twin by the same proportion.\nIf the cost is intentional, re-baseline "
            "with: python3 scripts/perf/board_perf.py --update",
            file=sys.stderr,
        )

    if stale:
        print("\nbaselines are stale (measured far below them):", file=sys.stderr)
        for entry in stale:
            print(
                f"  {entry['board']}: baseline {entry['baseline']:.1f} vs measured "
                f"{entry['measured']:.1f} Ir/step ({entry['delta']:+.1%})",
                file=sys.stderr,
            )
        print(
            "\nThat gap is dead gate: each of these boards can regress by it "
            "before anyone is told.\nLock the win in with: python3 "
            "scripts/perf/board_perf.py --update",
            file=sys.stderr,
        )

    if not ok:
        return 1

    print("\nno throughput regression")
    return 0


if __name__ == "__main__":
    sys.exit(main())
