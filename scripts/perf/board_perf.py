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
    All of them — every chip descriptor in `configs/chips/`, across four
    memory maps on Cortex-M plus the RISC-V and Xtensa ESP parts.

    Coverage is *derived* from the descriptors, not from a hand-kept list,
    because a hand-kept list silently stops covering whatever is added after it
    was last edited — which is how stm32f405, stm32f411ceu6, stm32f767 and
    rp2350 ended up outside the gate without even appearing in its "not
    covered" note. A chip that no fixture matches is a hard error, so adding a
    chip forces a decision rather than a silent gap.

    The Xtensa fixture needs the esp toolchain (espup). Where it is absent the
    two ESP32 parts are reported as NOT measured rather than quietly dropped,
    and --require-all (what CI passes) turns that into a failure.

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
from typing import NamedTuple

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
BASELINE_PATH = Path(__file__).resolve().parent / "baselines.json"
CHIP_DIR = REPO_ROOT / "configs/chips"

FIXTURE_DIR = REPO_ROOT / "target/perf-fixtures"


class Spin(NamedTuple):
    """How to build one flavour of the spin-loop fixture.

    `crate` is a workspace package built with `-p` when `directory` is None,
    and a standalone crate built from `directory` otherwise — the Xtensa
    fixture needs its own `.cargo/config.toml` (build-std, linkall.x), which
    only applies when cargo runs inside that directory.

    `optional` marks a fixture whose toolchain is not on a stock image: it may
    be skipped with a loud note rather than taking the whole gate down.
    """

    crate: str
    target: str
    toolchain: str | None = None
    features: str | None = None
    directory: str | None = None
    optional: bool = False
    # Xtensa placement comes from esp-hal's linkall.x per chip feature, not
    # from a memory.x this gate generates.
    env_origins: bool = True


# The spin loop, one crate per ISA. Within an ISA the source is identical, so a
# board's number tracks its own history exactly; across ISAs it does not, and
# cannot — a different instruction mix per simulated step is not something
# re-linking can normalise away. The gate is per-board-over-time either way.
SPIN_CORTEX_M = Spin("firmware-perf-spin", "thumbv6m-none-eabi")
SPIN_RISCV = Spin("firmware-perf-spin-riscv", "riscv32imc-unknown-none-elf")
# Xtensa is not a rustup target: it needs the esp-rs LLVM fork, which espup
# installs as the `esp` toolchain. CI installs it (core-nightly already does
# this for the S3 fixtures); a developer's machine usually has not, so these
# are optional and skip loudly instead of failing.
SPIN_XTENSA_ESP32 = Spin(
    crate="perf-spin-xtensa",
    target="xtensa-esp32-none-elf",
    toolchain="esp",
    features="esp32",
    directory="crates/firmware-perf-spin-xtensa",
    optional=True,
    env_origins=False,
)
SPIN_XTENSA_ESP32S3 = SPIN_XTENSA_ESP32._replace(
    target="xtensa-esp32s3-none-elf", features="esp32s3"
)

# One linked image per (arch, flash base, RAM base), read from the chip
# descriptor, so a chip is matched to a fixture by what it actually models
# rather than by name. Arch is part of the key because two ISAs can share a
# flash origin (the C3 and the S3 both boot at 0x42000000).
FIXTURES = {
    ("arm", 0x08000000, 0x20000000): ("stm32", SPIN_CORTEX_M),
    ("arm", 0x00000000, 0x20000000): ("nrf", SPIN_CORTEX_M),
    ("arm", 0x00000000, 0x1FFF8000): ("kinetis", SPIN_CORTEX_M),
    ("arm", 0x10000000, 0x20000000): ("rp2xxx", SPIN_CORTEX_M),
    ("riscv", 0x42000000, 0x3FC80000): ("esp32c3", SPIN_RISCV),
    ("xtensa-lx6", 0x400D0000, 0x3FFB0000): ("esp32", SPIN_XTENSA_ESP32),
    ("xtensa-lx7", 0x42000000, 0x3FC88000): ("esp32s3", SPIN_XTENSA_ESP32S3),
}

# Chips the gate cannot measure at all, with the reason. Anything here is
# reported on every run; anything neither here nor matched to a fixture aborts
# the run. Empty is the goal, not merely the current state — a chip belongs
# here only while there is a concrete reason it cannot be linked or run.
WAIVED: dict[str, str] = {}

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
    try:
        key = (chip["arch"], int(chip["flash"]["base"]), int(chip["ram"]["base"]))
    except (KeyError, TypeError, ValueError):
        return None
    entry = FIXTURES.get(key)
    return entry[0] if entry else None


def fixture_spec(name: str) -> Spin:
    """The build recipe behind a fixture name."""
    for fixture_name, spec in FIXTURES.values():
        if fixture_name == name:
            return spec
    raise KeyError(name)


def fixture_origins(name: str) -> tuple[int, int]:
    """The (flash, RAM) origins a fixture links against."""
    for (_arch, flash, ram), (fixture_name, _spec) in FIXTURES.items():
        if fixture_name == name:
            return flash, ram
    raise KeyError(name)


def toolchain_available(spec: Spin) -> bool:
    """Whether the toolchain this fixture needs is installed."""
    cmd = ["cargo"]
    if spec.toolchain:
        cmd.append(f"+{spec.toolchain}")
    cmd += ["--version"]
    try:
        if subprocess.run(cmd, capture_output=True).returncode != 0:
            return False
    except OSError:
        return False
    if spec.toolchain:
        # The esp toolchain carries its Xtensa targets in-tree; `rustup target
        # list` cannot see them, so the toolchain's presence is the signal.
        return True
    installed = subprocess.run(
        ["rustup", "target", "list", "--installed"], capture_output=True, text=True
    )
    return spec.target in installed.stdout.split()


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


def build_fixtures(fixtures: set[str]) -> tuple[dict[str, Path], dict[str, str]]:
    """Link the spin loop once per memory map.

    Returns ({fixture: ELF path}, {fixture: reason it was skipped}).
    """
    FIXTURE_DIR.mkdir(parents=True, exist_ok=True)
    built: dict[str, Path] = {}
    skipped: dict[str, str] = {}
    for name in sorted(fixtures):
        spec = fixture_spec(name)
        if not toolchain_available(spec):
            reason = f"toolchain for {spec.target} is not installed" + (
                f" (run `espup install` for the `{spec.toolchain}` toolchain)"
                if spec.toolchain
                else ""
            )
            if not spec.optional:
                raise RuntimeError(f"fixture '{name}': {reason}")
            skipped[name] = reason
            print(f"SKIPPING fixture '{name}': {reason}", file=sys.stderr)
            continue

        flash, ram = fixture_origins(name)
        out = FIXTURE_DIR / f"perf-spin-{name}"
        print(
            f"building fixture '{name}' ({spec.crate} {spec.target} "
            f"flash={flash:#010x} ram={ram:#010x})",
            file=sys.stderr,
        )

        cwd = REPO_ROOT / spec.directory if spec.directory else REPO_ROOT
        cmd = ["cargo"]
        if spec.toolchain:
            cmd.append(f"+{spec.toolchain}")
        cmd += ["build", "--release", "--target", spec.target]
        if not spec.directory:
            cmd += ["-p", spec.crate]
        if spec.features:
            cmd += ["--features", spec.features]

        env = dict(os.environ)
        if spec.env_origins:
            env["LABWIRED_PERF_FLASH_ORIGIN"] = f"{flash:#010x}"
            env["LABWIRED_PERF_RAM_ORIGIN"] = f"{ram:#010x}"

        proc = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True)
        if proc.returncode != 0:
            # A fixture on an optional toolchain that fails to build is
            # reported and skipped; anything on the stock toolchain is a real
            # break and takes the gate down, because silently not measuring the
            # STM32 boards is the failure this whole file exists to prevent.
            if not spec.optional:
                raise RuntimeError(
                    f"fixture '{name}' failed to build:\n{proc.stderr[-2000:]}"
                )
            skipped[name] = f"fixture build failed: {proc.stderr.strip().splitlines()[-1:]}"
            print(f"SKIPPING fixture '{name}': build failed\n{proc.stderr[-2000:]}", file=sys.stderr)
            continue

        elf = cwd / f"target/{spec.target}/release/{spec.crate}"
        if not elf.exists():
            elf = REPO_ROOT / f"target/{spec.target}/release/{spec.crate}"
        shutil.copy2(elf, out)
        built[name] = out
    return built, skipped


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
        "--require-all",
        action="store_true",
        help="fail if any covered board could not be measured (missing toolchain, "
        "failed fixture build) — what CI uses, so a skipped chip is never green",
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

    firmware, skipped_fixtures = build_fixtures({covered[b] for b in boards})
    baselines = json.loads(BASELINE_PATH.read_text()) if BASELINE_PATH.exists() else {}

    # A board whose toolchain is missing is not measured, and must not be
    # silently dropped: it is reported below and, when it was asked for by
    # name, it fails rather than passing on a measurement that never happened.
    skipped_boards = {
        b: skipped_fixtures[covered[b]] for b in boards if covered[b] in skipped_fixtures
    }
    boards = [b for b in boards if b not in skipped_boards]

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
    print(f"measured this run: {len(measured)}")
    if skipped_boards:
        print("NOT measured this run (toolchain missing on this machine):")
        for board, reason in sorted(skipped_boards.items()):
            print(f"  {board}: {reason}")
    if waived:
        print("not covered by this gate at all:")
        for board, reason in sorted(waived.items()):
            print(f"  {board}: {reason}")

    # A skipped board is only tolerable on a developer machine that simply
    # lacks an optional toolchain. Asking for it by name, or running under
    # --require-all as CI does, and getting silence back is precisely the
    # failure mode this gate exists to not have.
    strict = bool(args.boards) or args.require_all
    named_and_skipped = strict and bool(skipped_boards)
    ok = not regressions and not stale and not named_and_skipped
    if args.status_json:
        Path(args.status_json).write_text(
            json.dumps(
                {
                    "ok": ok,
                    "regressions": regressions,
                    "stale": stale,
                    "covered": {b: covered[b] for b in boards},
                    "skipped": skipped_boards,
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

    if named_and_skipped:
        print(
            "\ncovered boards that could not be measured here:",
            file=sys.stderr,
        )
        for board, reason in sorted(skipped_boards.items()):
            print(f"  {board}: {reason}", file=sys.stderr)

    if not ok:
        return 1

    print("\nno throughput regression")
    return 0


if __name__ == "__main__":
    sys.exit(main())
