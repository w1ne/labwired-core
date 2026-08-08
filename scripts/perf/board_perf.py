#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Per-board simulator throughput gate.

WHAT IT MEASURES
    Host instructions retired per simulated CPU step ("Ir/step"), for each
    board in the matrix, running the same fixture firmware
    (`crates/firmware-perf-spin`, a bare ALU spin loop), in each EXECUTION MODE
    that board's CLI driver has.

WHY TWO MODES AND NOT ONE
    "Ir/step" is only the number that transfers to users if it is measured on
    the loop users run. There are two, and they are not the same loop:

      step   `Machine::step()` — one instruction per call, the CLI default on
             ARM and the only thing the Xtensa driver has.
      batch  `Machine::advance(AdvanceRequest::run(..))` — the call the browser
             makes from `Sim::step_batch` in `crates/wasm/src/lib.rs`, and the
             CLI default on RISC-V.

    This gate used to measure only `step` on ARM, and that made its own stated
    rationale false there. #830 removed three clamps that had pinned the ARM CPU
    quantum to one instruction, worth 9-16x native throughput on the batched
    path — and all 22 ARM boards moved 0.2-0.4% through this gate, because the
    gate never entered that path. A regression in ARM batch orchestration (a
    fourth clamp of the kind #830 deleted) was invisible on the only path the
    browser runs.

    Which modes a board has is derived from its fixture's `Spin.modes`, not
    hand-listed per board, for the same reason coverage is derived from the chip
    descriptors: a hand-kept list stops covering whatever is added after it.
    `batch` is measured by passing `--batched` to `labwired run`, which is an
    assertion rather than a hint — the CLI fails the run instead of falling back
    to single-stepping, and prints a `[batched] instructions=.. batches=..` line
    this gate requires as proof of which loop executed.

    The `batch` mode's absolute noise floor is the same as `step`'s (about
    ±0.5 Ir/step run to run on the same binary), but it sits on a number ~10x
    smaller, so its RELATIVE reproducibility is ~±0.5% rather than ~±0.03%
    (measured: stm32l476 batch over four runs 203.6 / 203.8 / 204.3 / 204.6).
    Still 6x inside the 3% tolerance, but a `batch` delta under 1% is noise and
    should not be read as a finding.

    Note that batching engaged is not the same as batching WIDE. A bus carrying
    something non-relaxable (H5 embedded FLASH modelling its own ops, nRF54L15's
    non-walk-deletable peripherals) reports `max_safe_tick_interval() == 1` or
    `requires_cycle_accurate()`, so its batches are one instruction wide and its
    `batch` number lands near its `step` number. That is a true property of the
    board, reported as `steps_per_batch=1.00`, not a failure to measure.

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
    Each board is run twice per mode, at two different step counts, and the
    per-step cost is the SLOPE between them. ELF loading, YAML parsing and
    simulator construction are identical in both runs, so they cancel out and
    never pollute the number. The `[batched]` summary line is one `eprintln!`
    on exit, identical in both runs, so it cancels too — and the gate checks
    that both runs retired exactly the requested instruction count, because a
    slope whose denominator is assumed rather than confirmed is not a
    measurement.

WHICH BOARDS ARE MATCHED TO A FIXTURE
    Every chip descriptor in `configs/chips/` is matched to a linked fixture by
    (arch, flash base, RAM base) read from the descriptor itself, and to that
    fixture's modes.

    The match is *derived* from the descriptors, not from a hand-kept list,
    because a hand-kept list silently stops covering whatever is added after it
    was last edited — which is how stm32f405, stm32f411ceu6, stm32f767 and
    rp2350 ended up outside the gate without even appearing in its "not
    covered" note. A chip that no fixture matches is a hard error, so adding a
    chip forces a decision rather than a silent gap.

A MATCHED FIXTURE IS NOT A MEASUREMENT
    A fixture nothing has ever built produces no number, and a board-mode with
    no number is not guarded by anything — whatever the matching table says.
    Reporting "covers N chips" off the match alone is the same class of
    overstatement this gate exists to catch: a check that reads complete while
    measuring nothing. So every report here keeps three states apart, and
    derives all three from state rather than from a second hand-kept list:

      measured this run        a number was produced, here, in this run.
      skipped this run         no number here — the fixture's toolchain is not
                               installed on this machine — but a baseline is on
                               record, so some run somewhere did measure it.
      never measured anywhere  no baseline exists for this board-mode and this
                               run did not produce one either. No run has ever
                               produced a number for it. This is NOT coverage
                               and is never counted as coverage.

    The unit of all three is the BOARD-MODE, not the board, for the same reason
    the gate itself is per-mode: `stm32l476` having a `step` baseline says
    nothing about whether its `batch` loop is measured, and #830 is the standing
    proof that a per-board count hides exactly that.

    "Has this ever been measured" is answered by baselines.json, for the same
    reason the match set is answered by configs/chips/: a baseline is the
    residue of a real measurement, so it cannot claim a run that did not happen
    and it cannot drift out of date the way a hand-kept "not covered" note does.

    The three Xtensa parts (esp32, esp32s3, esp32s3-zero) sit in that third
    state today: `crates/firmware-perf-spin-xtensa` needs the esp-rs toolchain
    (espup) and has not been built by any run — CI's espup step is
    continue-on-error and baselines.json has no entry for them in any mode. They
    are named as NEVER measured on every run, and --require-all (what CI passes)
    fails rather than reporting them green. They were previously in WAIVED,
    which said so honestly; moving them into FIXTURES made them read as covered,
    which is what this wording exists to prevent recurring.

WHY A BASELINE THAT IS TOO HIGH ALSO FAILS
    A board that measures far *below* its baseline is not good news, it is a
    dead gate: the slack is exactly how much it can regress before anyone is
    told. Improvements have to be locked in with --update, same as accepted
    costs.

BASELINE SCHEMA
    `{board: {mode: Ir/step}}`. Nesting rather than flattening to `board.mode`
    keys keeps "which boards are baselined" a plain `set(baselines)` question,
    which is what the coverage tests ask, and makes a board gaining or losing a
    mode a visible diff instead of a silent one.

USAGE
    python3 scripts/perf/board_perf.py                 # check against baselines
    python3 scripts/perf/board_perf.py --update        # rewrite baselines
    python3 scripts/perf/board_perf.py --boards stm32f103,stm32l476
    python3 scripts/perf/board_perf.py --modes batch   # one mode only
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

# The two execution loops `labwired run` can drive a Machine with. `step` is
# `Machine::step()`, one instruction per call. `batch` is
# `Machine::advance(AdvanceRequest::run(..))`, which is what the browser calls
# and where the ARM batch-orchestration cost lives.
MODE_STEP = "step"
MODE_BATCH = "batch"
ALL_MODES = (MODE_STEP, MODE_BATCH)

# Marker the CLI prints on exit under `--batched`. Its absence means the run did
# not take the batched loop, which must fail the measurement rather than be
# recorded as one.
BATCHED_RE = re.compile(
    r"^\[batched\] instructions=(\d+) batches=(\d+) steps_per_batch=([\d.]+) "
    r"tick_interval=(\d+)",
    re.MULTILINE,
)


class Spin(NamedTuple):
    """How to build one flavour of the spin-loop fixture.

    `crate` is a workspace package built with `-p` when `directory` is None,
    and a standalone crate built from `directory` otherwise — the Xtensa
    fixture needs its own `.cargo/config.toml` (build-std, linkall.x), which
    only applies when cargo runs inside that directory.

    `optional` marks a fixture whose toolchain is not on a stock image: it may
    be skipped with a loud note rather than taking the whole gate down.

    `modes` is the set of execution loops `labwired run` has for this ISA. It
    lives here rather than in a per-board table because it is a property of the
    CLI driver, one driver per ISA, and a per-board table would stop covering
    boards added after it was last edited.
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
    modes: tuple[str, ...] = (MODE_STEP, MODE_BATCH)


# The spin loop, one crate per ISA. Within an ISA the source is identical, so a
# board's number tracks its own history exactly; across ISAs it does not, and
# cannot — a different instruction mix per simulated step is not something
# re-linking can normalise away. The gate is per-board-over-time either way.
#
# Modes: the Cortex-M driver has both loops (`step` is its default, `batch` is
# behind `--batched`). The RISC-V driver already batches by default — #830's gap
# is why esp32c3 was unaffected — so `batch` is the only loop it has that is not
# an instrumentation mode. The Xtensa driver never builds a `Machine` at all; it
# runs `cpu.step()` + `tick_peripherals_with_costs()` directly, so `step` is all
# there is and `--batched` is rejected there rather than silently ignored.
SPIN_CORTEX_M = Spin("firmware-perf-spin", "thumbv6m-none-eabi", modes=ALL_MODES)
SPIN_RISCV = Spin(
    "firmware-perf-spin-riscv",
    "riscv32imc-unknown-none-elf",
    modes=(MODE_BATCH,),
)
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
    modes=(MODE_STEP,),
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

# Chips no fixture can even be LINKED for, with the reason. Anything here is
# reported on every run; anything neither here nor matched to a fixture aborts
# the run. Empty is the goal, not merely the current state — a chip belongs
# here only while there is a concrete reason it cannot be linked or run.
#
# This is deliberately NOT where "matched but never actually measured" is
# recorded. That state is derived from baselines.json (never_measured_board_modes)
# precisely so it cannot be dropped from here and start reading as coverage —
# which is what happened when the Xtensa parts were moved out of this dict into
# FIXTURES and WAIVED was emptied.
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


class ModeNotTakenError(RuntimeError):
    """A run did not execute the loop it was asked to measure.

    Raised rather than warned: a `batch` number produced by the single-step
    loop would read as a passing gate over a path nobody measured, which is the
    exact failure the two-mode split exists to remove.
    """


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


def load_baselines() -> dict[str, dict[str, float]]:
    """The baseline file, as `{board: {mode: Ir/step}}`.

    The pre-mode schema was `{board: Ir/step}`. It is rejected rather than
    coerced, because there is no correct coercion: the same bare number meant
    the single-step loop on ARM and the batched loop on RISC-V. Guessing would
    mislabel one of them and gate the wrong path.
    """
    if not BASELINE_PATH.exists():
        return {}
    raw = json.loads(BASELINE_PATH.read_text())
    flat = sorted(b for b, v in raw.items() if not isinstance(v, dict))
    if flat:
        raise ValueError(
            f"{BASELINE_PATH.name} uses the pre-mode schema for: {', '.join(flat)}\n"
            "The schema is now {board: {mode: Ir/step}} — see this file's "
            "BASELINE SCHEMA note. Re-measure with "
            "`python3 scripts/perf/board_perf.py --update` from a known-good "
            "commit rather than hand-converting: which loop the old number came "
            "from is not recoverable from the number."
        )
    return raw


def fixture_spec(name: str) -> Spin:
    """The build recipe behind a fixture name."""
    for fixture_name, spec in FIXTURES.values():
        if fixture_name == name:
            return spec
    raise KeyError(name)


def modes_for(fixture: str) -> tuple[str, ...]:
    """Which execution loops this fixture's ISA driver actually has."""
    return fixture_spec(fixture).modes


def board_modes(covered: dict[str, str]) -> list[tuple[str, str]]:
    """Every (board, mode) the fixture table matches.

    The board-mode, not the board, is the unit every count in this file is
    expressed in. A board is not a unit of measurement here: stm32l476 having a
    `step` baseline says nothing about whether its `batch` loop has ever been
    run, and #830 is the standing proof that a per-board tally hides precisely
    that half.

    Ordered board-alphabetically, then in the fixture's declared mode order, so
    the summary lists a board-mode in the same place the measurement table above
    it does — a reader comparing "measured" against "never measured" is reading
    two views of one list, not two differently-sorted lists.
    """
    return [(b, m) for b, f in sorted(covered.items()) for m in modes_for(f)]


def has_been_measured(board: str, mode: str, baselines: dict, measured: dict | None = None) -> bool:
    """Whether a number for this board-mode exists ANYWHERE.

    Two sources, and only two, both of them evidence of a run that actually
    happened rather than of an intention to run: a recorded baseline (some run,
    some machine, at some point) and `measured` (this run, just now). A fixture
    match is deliberately not a source — it is a build recipe, and a recipe
    nobody has cooked is not a meal.
    """
    return mode in baselines.get(board, {}) or mode in (measured or {}).get(board, {})


def never_measured_board_modes(
    covered: dict[str, str], baselines: dict, measured: dict | None = None
) -> list[tuple[str, str]]:
    """Matched board-modes for which no number has ever existed, anywhere.

    These are NOT coverage, and are never folded into a coverage count. A
    board-mode here has a linkable fixture and nothing else: with no baseline
    there is nothing to compare against, so no regression in it is detectable
    and nothing about it is being held still.
    """
    return [
        bm for bm in board_modes(covered) if not has_been_measured(*bm, baselines, measured)
    ]


def bm_label(board: str, mode: str) -> str:
    """How a board-mode is named in output, matching the per-row `[mode]` form."""
    return f"{board}[{mode}]"


def never_measured_note(board: str, mode: str, fixture: str) -> str:
    """Why this matched board-mode has still never produced a number.

    Derived from the fixture's own build recipe rather than from a written-down
    reason, so it cannot go stale the way a hand-kept waiver note does — the
    note stops being emitted the moment a baseline appears, because the
    board-mode leaves the set.
    """
    spec = fixture_spec(fixture)
    if spec.toolchain:
        return (
            f"fixture '{fixture}' needs the `{spec.toolchain}` toolchain "
            f"({spec.target}), which no run has had; nothing has ever built it"
        )
    return f"fixture '{fixture}' ({spec.target}) has never produced a number in this mode"


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


class Run(NamedTuple):
    """One callgrind measurement and what the CLI said about it."""

    irefs: int
    # Instructions the simulator reports it actually retired, and how wide its
    # CPU dispatch batches were. Only the batched loop reports these; on the
    # single-step loop the width is 1 by construction.
    instructions: int | None = None
    steps_per_batch: float | None = None
    tick_interval: int | None = None


def measure_once(cli: Path, chip: Path, firmware: Path, steps: int, mode: str) -> Run:
    """Retired host instructions for a full run of `steps` simulated steps."""
    # `--batched` is the only difference between the two modes: same binary,
    # same fixture, same step count, so anything the slope shows is the loop.
    extra = ["--batched"] if mode == MODE_BATCH else []
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
                *extra,
            ],
            capture_output=True,
            text=True,
        )
    match = IREFS_RE.search(proc.stderr)
    if not match:
        raise RuntimeError(
            f"callgrind produced no instruction count for {chip.name} "
            f"[{mode}]:\n{proc.stderr[-2000:]}"
        )
    irefs = int(match.group(1).replace(",", ""))

    if mode != MODE_BATCH:
        return Run(irefs)

    # The whole point of the `batch` mode is that it is a DIFFERENT loop. If the
    # CLI did not print its proof-of-path line, we have a number for the loop we
    # were trying not to measure, and reporting it would be worse than reporting
    # nothing.
    proof = BATCHED_RE.search(proc.stderr)
    if not proof:
        raise ModeNotTakenError(
            f"{chip.stem}: asked for the batched loop but the CLI printed no "
            f"'[batched] ...' line, so the run did not take it. Is the CLI "
            f"older than `--batched`?\n{proc.stderr[-2000:]}"
        )
    instructions = int(proof.group(1))
    if instructions != steps:
        # The slope's denominator is (STEPS_HIGH - STEPS_LOW). That is only the
        # simulated work done if each run retired exactly what it was asked for
        # — a run that halted early, or that spent fuel on idle fast-forward
        # rather than instructions, would silently inflate Ir/step.
        raise ModeNotTakenError(
            f"{chip.stem}: batched run was asked for {steps} steps but retired "
            f"{instructions}; the slope denominator would be wrong"
        )
    return Run(irefs, instructions, float(proof.group(3)), int(proof.group(4)))


class Measurement(NamedTuple):
    """A board's Ir/step in one mode, plus how it was executed."""

    ir_per_step: float
    steps_per_batch: float | None = None
    tick_interval: int | None = None


def measure_board(cli: Path, board: str, firmware: Path, mode: str) -> Measurement:
    chip = CHIP_DIR / f"{board}.yaml"
    if not chip.exists():
        raise FileNotFoundError(f"no chip descriptor for board '{board}': {chip}")
    low = measure_once(cli, chip, firmware, STEPS_LOW, mode)
    high = measure_once(cli, chip, firmware, STEPS_HIGH, mode)
    ir_per_step = (high.irefs - low.irefs) / (STEPS_HIGH - STEPS_LOW)
    return Measurement(ir_per_step, high.steps_per_batch, high.tick_interval)


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
    parser.add_argument(
        "--modes",
        help="comma-separated subset of execution modes to measure "
        f"({', '.join(ALL_MODES)}); default: every mode each board's driver has. "
        "Halves the run while iterating; not for CI, which needs both",
    )
    args = parser.parse_args()

    modes_filter = set(args.modes.split(",")) if args.modes else None
    if modes_filter and not modes_filter <= set(ALL_MODES):
        print(
            f"error: unknown mode(s): {', '.join(sorted(modes_filter - set(ALL_MODES)))}",
            file=sys.stderr,
        )
        return 2

    if args.check_coverage:
        try:
            covered, waived = plan_coverage(discover_chips())
        except CoverageError as exc:
            print(f"error: {exc}", file=sys.stderr)
            return 1
        try:
            baselines = load_baselines()
        except ValueError as exc:
            print(f"error: {exc}", file=sys.stderr)
            return 1
        matched = board_modes(covered)
        never = never_measured_board_modes(covered, baselines)
        gated = [bm for bm in matched if bm not in set(never)]
        chips_with_a_number = {b for b, _ in gated}

        # This is the line the PR gate prints, and the first number on it is the
        # only one most people read — so it is the honest one: board-modes with
        # a baseline, i.e. the ones a regression could actually be detected in.
        # The old wording ("perf gate covers 25 chips ... (46 board-modes)")
        # counted three board-modes that have never produced a number anywhere
        # as covered, and that count is what went into a PR description as
        # though it meant 25 chips were being measured.
        print(
            f"perf gate: {len(gated)} of {len(matched)} board-modes have a measured "
            f"baseline ({len(chips_with_a_number)} of {len(covered)} chips); "
            f"{len(never)} NEVER measured anywhere; {len(waived)} waived"
        )
        print(
            f"  fixtures matched: {len(covered)} chips / "
            f"{len(set(covered.values()))} memory maps / {len(matched)} board-modes "
            "— a matched fixture is a build recipe, not a measurement"
        )
        if never:
            print(
                f"  NEVER measured anywhere ({len(never)}) — no baseline exists, so "
                "nothing is being held still for these:"
            )
            for board, mode in never:
                print(f"    {bm_label(board, mode)}: {never_measured_note(board, mode, covered[board])}")
            starved = sorted({b for b, _ in never} - chips_with_a_number)
            if starved:
                # A chip with no measured mode AT ALL is the strongest form of
                # the overstatement: it appears in the matched-chip count while
                # nothing anywhere has ever run it.
                print(
                    f"  chips with no measured mode at all ({len(starved)}): "
                    f"{', '.join(starved)}"
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

    # Kept before the skip filter below eats into `boards`, so the summary can
    # tell "this machine could not measure it" apart from "nobody asked for it".
    requested = list(boards)

    firmware, skipped_fixtures = build_fixtures({covered[b] for b in boards})
    try:
        baselines = load_baselines()
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    # A board whose toolchain is missing is not measured, and must not be
    # silently dropped: it is reported below and, when it was asked for by
    # name, it fails rather than passing on a measurement that never happened.
    skipped_boards = {
        b: skipped_fixtures[covered[b]] for b in boards if covered[b] in skipped_fixtures
    }
    boards = [b for b in boards if b not in skipped_boards]

    measured: dict[str, dict[str, float]] = {}
    regressions: list[dict] = []
    stale: list[dict] = []
    # A mode that could not be executed. Never dropped: it fails the run, in
    # --update too, because re-baselining around an unmeasurable mode is how a
    # gate quietly loses a path.
    unmeasurable: list[str] = []
    print(
        f"{'board':<16} {'fixture':<9} {'mode':<6} {'Ir/step':>10} "
        f"{'baseline':>10} {'delta':>9}  {'batch':>6}"
    )
    print("-" * 74)
    for board in boards:
        fixture = covered[board]
        for mode in modes_for(fixture):
            if modes_filter and mode not in modes_filter:
                continue
            try:
                m = measure_board(cli, board, firmware[fixture], mode)
            except ModeNotTakenError as exc:
                print(f"{board:<16} {fixture:<9} {mode:<6} {'NOT TAKEN':>10}")
                unmeasurable.append(str(exc))
                continue
            measured.setdefault(board, {})[mode] = round(m.ir_per_step, 1)
            # `steps_per_batch` is not gated — it is a property of the bus, not
            # of engine cost — but it is printed, because a batch mode sitting
            # at 1.00 is the difference between "this board batches" and "this
            # board has a non-relaxable peripheral".
            width = (
                f"{m.steps_per_batch:>6.1f}" if m.steps_per_batch is not None else " " * 6
            )
            base = baselines.get(board, {}).get(mode)
            if base is None:
                print(
                    f"{board:<16} {fixture:<9} {mode:<6} {m.ir_per_step:>10.1f} "
                    f"{'(new)':>10} {'':>9}  {width}"
                )
                continue
            delta = (m.ir_per_step - base) / base
            entry = {
                "board": board,
                "mode": mode,
                "baseline": round(base, 1),
                "measured": round(m.ir_per_step, 1),
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
                f"{board:<16} {fixture:<9} {mode:<6} {m.ir_per_step:>10.1f} "
                f"{base:>10.1f} {delta:>+8.1%}  {width}{flag}"
            )

    # ── Summary ───────────────────────────────────────────────────────────
    # Three states, at board-mode granularity, never collapsed into one
    # "covered" number. The line that matters is between a board-mode that has
    # a number (from this run or an earlier one) and a board-mode for which no
    # number has ever existed: only the first kind can regress detectably, so
    # only the first kind is coverage. `measured` is passed in, so a board-mode
    # measured here for the first time counts as measured rather than as never
    # — but nothing else promotes it.
    matched = board_modes(covered)
    never = never_measured_board_modes(covered, baselines, measured)
    never_set = set(never)
    # A board-mode not measured here that DOES have a baseline: some machine
    # with that toolchain measured it, this one could not. Honest middle state.
    skipped_bms = [
        (b, m)
        for b in sorted(skipped_boards)
        for m in modes_for(covered[b])
        if (b, m) not in never_set
    ]
    measured_bms = sum(len(v) for v in measured.values())

    print()
    print(
        f"measured this run: {measured_bms} board-modes across {len(measured)} chips / "
        f"{len({covered[b] for b in measured})} memory maps"
    )
    if unmeasurable:
        print("MODES THAT DID NOT EXECUTE (measured nothing):")
        for note in unmeasurable:
            print(f"  {note.splitlines()[0]}")
    if skipped_bms:
        print(
            f"skipped this run ({len(skipped_bms)}) — not measured here, but a baseline "
            "from an earlier run is on record:"
        )
        for board, mode in skipped_bms:
            print(f"  {bm_label(board, mode)}: {skipped_boards[board]}")
    if never:
        # Deliberately not folded into any total above. These board-modes have
        # a fixture recipe and nothing else — no baseline, so no regression in
        # them is detectable and nothing about them is being held still.
        print(
            f"NEVER measured anywhere ({len(never)}) — no baseline exists and this run "
            "produced none, so these are NOT covered by this gate:"
        )
        for board, mode in never:
            # Prefer this run's concrete reason (toolchain absent, build broke)
            # over the generic one when there is one.
            reason = skipped_boards.get(board) or never_measured_note(board, mode, covered[board])
            print(f"  {bm_label(board, mode)}: {reason}")
    if waived:
        print("no fixture can be linked at all (waived):")
        for board, reason in sorted(waived.items()):
            print(f"  {board}: {reason}")
    not_requested = sorted(set(covered) - set(requested))
    if not_requested:
        print(f"not requested this run ({len(not_requested)} chips): {', '.join(not_requested)}")
    # Last line of the block, so none of the arithmetic above it is mistaken
    # for coverage: matching is what the fixture table does, measuring is what
    # the gate does, and they are different numbers.
    print(
        f"fixtures matched {len(matched)} board-modes across {len(covered)} chips / "
        f"{len(set(covered.values()))} memory maps — matching is not measuring"
    )

    # A skipped board is only tolerable on a developer machine that simply
    # lacks an optional toolchain. Asking for it by name, or running under
    # --require-all as CI does, and getting silence back is precisely the
    # failure mode this gate exists to not have.
    strict = bool(args.boards) or args.require_all
    named_and_skipped = strict and bool(skipped_boards)
    ok = not regressions and not stale and not named_and_skipped and not unmeasurable
    if args.status_json:
        Path(args.status_json).write_text(
            json.dumps(
                {
                    "ok": ok,
                    "regressions": regressions,
                    "stale": stale,
                    # `matched` is what the fixture table pairs up; it is NOT a
                    # coverage claim. The CI issue body quotes the report text
                    # verbatim, so the same three-way split has to exist here or
                    # a consumer reading the JSON reconstructs the overstatement
                    # the text no longer makes.
                    "matched": {
                        b: {"fixture": covered[b], "modes": modes_for(covered[b])}
                        for b in boards
                    },
                    "skipped": skipped_boards,
                    "never_measured": [
                        {"board": b, "mode": m, "fixture": covered[b]} for b, m in never
                    ],
                    "unmeasurable": unmeasurable,
                    "waived": waived,
                    "measured": measured,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n"
        )

    if unmeasurable:
        print(
            "\nthese modes did not execute, so nothing was measured for them:",
            file=sys.stderr,
        )
        for note in unmeasurable:
            print(f"  {note}", file=sys.stderr)
        # Deliberately BEFORE --update: writing a baseline file while a mode is
        # unmeasurable would bake the gap in and make the next run green.
        return 1

    if args.update:
        # Merge per board AND per mode, so `--boards x --modes batch` updates one
        # number rather than deleting that board's other modes.
        merged = {b: dict(v) for b, v in baselines.items()}
        for board, by_mode in measured.items():
            merged.setdefault(board, {}).update(by_mode)
        BASELINE_PATH.write_text(
            json.dumps(
                {b: dict(sorted(m.items())) for b, m in sorted(merged.items())}, indent=2
            )
            + "\n"
        )
        print(f"\nwrote {BASELINE_PATH.relative_to(REPO_ROOT)}")
        return 0

    if regressions:
        print("\nsimulator throughput regressed:", file=sys.stderr)
        for entry in regressions:
            print(
                f"  {entry['board']} [{entry['mode']}]: {entry['baseline']:.1f} -> "
                f"{entry['measured']:.1f} Ir/step ({entry['delta']:+.1%})",
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
                f"  {entry['board']} [{entry['mode']}]: baseline {entry['baseline']:.1f} "
                f"vs measured {entry['measured']:.1f} Ir/step ({entry['delta']:+.1%})",
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
