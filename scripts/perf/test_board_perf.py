# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Tests for the per-board perf gate's coverage planning and its reporting.

The measurement itself needs valgrind and a release CLI, so it is not unit
tested here. What is tested is the property that actually failed in practice:
that a chip in `configs/chips/` cannot end up outside the gate without anyone
being told — and its second half, which failed later and more quietly, that a
chip inside the gate cannot be REPORTED as covered while nothing has ever
measured it.

NOTE FOR WHOEVER ADDS THE NEXT FILE HERE: nothing discovers these tests.
`.github/workflows/core-ci.yml` (job `pr-gate`, step "Perf gate — every chip
covered or waived") names each test file explicitly, so a new file under
scripts/ runs nowhere until it is added to that list.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import board_perf as bp  # noqa: E402


def _chip(flash: int, ram: int, arch: str = "arm") -> dict:
    return {"arch": arch, "flash": {"base": flash}, "ram": {"base": ram}}


def test_stm32_map_is_covered():
    covered, waived = bp.plan_coverage({"stm32f103": _chip(0x08000000, 0x20000000)})
    assert covered == {"stm32f103": "stm32"}
    assert waived == {}


def test_batch_measurement_uses_three_slope_median(monkeypatch, tmp_path):
    monkeypatch.setattr(bp, "STEPS_LOW", 10)
    monkeypatch.setattr(bp, "STEPS_HIGH", 20)
    runs = iter(
        [
            bp.Run(100),
            bp.Run(140, 20, 500.0, 512),
            bp.Run(100),
            bp.Run(190, 20, 512.0, 512),
            bp.Run(100),
            bp.Run(160, 20, 504.0, 512),
        ]
    )
    monkeypatch.setattr(bp, "measure_once", lambda *args: next(runs))

    measured = bp.measure_board(
        Path("labwired"), "atsamd21", tmp_path / "spin.elf", bp.MODE_BATCH
    )

    assert measured.ir_per_step == 6.0
    assert measured.steps_per_batch == 504.0
    assert measured.tick_interval == 512


def test_batch_noise_floor_filters_sub_instruction_jitter_not_real_regressions():
    assert not bp.is_regression(3.8, 3.4, bp.MODE_BATCH)
    assert bp.is_regression(4.0, 3.4, bp.MODE_BATCH)
    assert bp.is_regression(900.0, 850.0, bp.MODE_STEP)


def test_batch_noise_floor_is_symmetric_for_stale_baselines():
    # A 0.4 Ir/step swing is the same runner noise whichever direction it has.
    assert not bp.is_stale(2.8, 3.2, bp.MODE_BATCH)
    assert bp.is_stale(2.6, 3.2, bp.MODE_BATCH)
    assert bp.is_stale(700.0, 850.0, bp.MODE_STEP)


def test_recommended_tick_interval_is_a_performance_ratchet():
    source = (bp.REPO_ROOT / "crates/core/src/bus/mod.rs").read_text()
    match = re.search(r"RECOMMENDED_TICK_INTERVAL:\s*u32\s*=\s*(\d+)", source)
    assert match, "RECOMMENDED_TICK_INTERVAL declaration moved or disappeared"
    assert int(match.group(1)) >= bp.MIN_RECOMMENDED_TICK_INTERVAL


def test_each_memory_map_has_its_own_fixture():
    chips = {
        "nrf52840": _chip(0x00000000, 0x20000000),
        "mkw41z4": _chip(0x00000000, 0x1FFF8000),
        "rp2040": _chip(0x10000000, 0x20000000),
        "stm32l476": _chip(0x08000000, 0x20000000),
    }
    covered, _ = bp.plan_coverage(chips)
    # Same flash base, different RAM base, must not share a fixture: the reset
    # stack pointer comes from the linked RAM origin, so the wrong one faults
    # before main() instead of measuring anything.
    assert covered["nrf52840"] != covered["mkw41z4"]
    assert len(set(covered.values())) == 4


def test_same_flash_base_across_isas_does_not_collide():
    """The C3 and the S3 both boot at 0x42000000 on different ISAs."""
    covered, _ = bp.plan_coverage(
        {
            "esp32c3": _chip(0x42000000, 0x3FC80000, arch="riscv"),
            "esp32s3": _chip(0x42000000, 0x3FC88000, arch="xtensa-lx7"),
        }
    )
    assert covered["esp32c3"] != covered["esp32s3"]
    assert bp.fixture_spec(covered["esp32c3"]).target.startswith("riscv32")
    assert bp.fixture_spec(covered["esp32s3"]).target.startswith("xtensa")


def test_every_fixture_declares_at_least_one_real_mode():
    """A fixture with no modes is covered on paper and measured never."""
    for _name, spec in bp.FIXTURES.values():
        assert spec.modes, f"{spec.crate} declares no execution modes"
        assert set(spec.modes) <= set(bp.ALL_MODES), spec.modes


def test_only_optional_fixtures_may_be_skipped():
    """A missing stock toolchain must fail, not degrade to a skip."""
    for _name, spec in bp.FIXTURES.values():
        if spec.toolchain is None:
            assert not spec.optional, f"{spec.crate} is on a stock toolchain but optional"
        else:
            assert spec.optional, f"{spec.crate} needs {spec.toolchain} but is not optional"


def test_unclassified_chip_is_an_error_not_a_silent_skip():
    with pytest.raises(bp.CoverageError) as exc:
        bp.plan_coverage({"newchip": _chip(0x60000000, 0x24000000)})
    assert "newchip" in str(exc.value)
    assert "0x60000000" in str(exc.value)


def test_unknown_riscv_map_is_an_error_not_a_silent_skip():
    """Matching is per (arch, flash, ram): a new RISC-V map is not covered."""
    with pytest.raises(bp.CoverageError):
        bp.plan_coverage({"someriscv": _chip(0x80000000, 0x80020000, arch="riscv")})


def test_waivers_are_explicit():
    """WAIVED must stay a deliberate shortlist — not a dumping ground.

    atmega328p USED to be here, waived for want of a firmware-perf-spin-avr
    crate. That crate now exists (built by avr-gcc rather than cargo, see
    Spin.builder), so the waiver is gone and the board is measured like any
    other. Any chip here is a regression that needs a fixture or a new
    documented reason.
    """
    _, waived = bp.plan_coverage(bp.discover_chips())
    assert "atmega328p" not in waived, (
        "the AVR spin fixture exists now; atmega328p must be measured, not waived"
    )
    assert set(waived) == {"rv1103"}, f"unexpected waivers (add a fixture): {waived}"


def test_rv1103_cpu_subset_is_waived_without_claiming_platform_performance():
    """A generic Cortex-A7 smoke ELF is not an RV1103 throughput fixture."""
    covered, waived = bp.plan_coverage({"rv1103": bp.discover_chips()["rv1103"]})
    assert covered == {}
    reason = waived["rv1103"]
    assert "Cortex-A7 CPU subset" in reason
    assert "RV1103 platform" in reason
    assert "boot" in reason and "Linux" in reason
    assert "representative perf fixture" in reason


def test_real_chip_tree_is_fully_classified():
    """The check the CI step runs: no chip in the tree is unaccounted for."""
    covered, waived = bp.plan_coverage(bp.discover_chips())
    assert covered, "no chips covered — discovery is broken"
    overlap = set(covered) & set(waived)
    assert not overlap, f"chips both covered and waived: {overlap}"


def test_every_covered_board_has_a_baseline():
    """A covered board with no baseline is measured but gates nothing."""
    import json

    covered, _ = bp.plan_coverage(bp.discover_chips())
    baselines = json.loads(bp.BASELINE_PATH.read_text())
    # Boards on an optional toolchain are exempt: their first measurement can
    # only come from a machine that has that toolchain, so the baseline lands
    # after CI's first run rather than with the code. They are still measured
    # and reported meanwhile — just as "(new)" rather than gated.
    gated = {b for b, f in covered.items() if not bp.fixture_spec(f).optional}
    missing = sorted(gated - set(baselines))
    assert not missing, (
        f"covered but unbaselined: {missing} — run "
        "`python3 scripts/perf/board_perf.py --update`"
    )


def test_every_covered_mode_has_a_baseline():
    """A board baselined in one mode still gates nothing in the other.

    The gap this closes: ARM had a `step` baseline and no `batch` one, so the
    batched orchestration the browser runs could regress freely while the gate
    stayed green on the loop nobody runs.
    """
    import json

    covered, _ = bp.plan_coverage(bp.discover_chips())
    baselines = json.loads(bp.BASELINE_PATH.read_text())
    missing = sorted(
        f"{board}[{mode}]"
        for board, fixture in covered.items()
        if not bp.fixture_spec(fixture).optional
        for mode in bp.modes_for(fixture)
        if mode not in baselines.get(board, {})
    )
    assert not missing, (
        f"covered but unbaselined: {missing} — run "
        "`python3 scripts/perf/board_perf.py --update`"
    )


def test_no_baseline_for_a_mode_a_board_does_not_have():
    """A leftover mode key reads as coverage the gate does not have."""
    import json

    covered, _ = bp.plan_coverage(bp.discover_chips())
    baselines = json.loads(bp.BASELINE_PATH.read_text())
    orphans = sorted(
        f"{board}[{mode}]"
        for board, by_mode in baselines.items()
        if board in covered
        for mode in by_mode
        if mode not in bp.modes_for(covered[board])
    )
    assert not orphans, f"baselines for modes that are not measured: {orphans}"


def test_baselines_are_per_mode_dicts():
    """The schema is `{board: {mode: Ir/step}}`, not a bare number.

    Guards the migration: a flat `{board: 1498.0}` entry left behind would be
    read as "no baseline for either mode" by `baselines.get(board, {}).get(mode)`
    and silently gate nothing.
    """
    import json

    baselines = json.loads(bp.BASELINE_PATH.read_text())
    for board, entry in baselines.items():
        assert isinstance(entry, dict), f"{board}: expected {{mode: Ir/step}}, got {entry!r}"
        assert entry, f"{board}: empty baseline entry"
        for mode, value in entry.items():
            assert mode in bp.ALL_MODES, f"{board}: unknown mode {mode!r}"
            assert isinstance(value, (int, float)), f"{board}[{mode}]: {value!r}"


def test_arm_boards_are_gated_on_the_path_the_browser_runs():
    """Every Cortex-M board must be measured in `batch`, not only in `step`.

    `Sim::step_batch` in crates/wasm calls `Machine::advance`; the CLI default
    for ARM calls `Machine::step`. Measuring only the latter is what made #830's
    9-16x batching win show up here as +0.2%.
    """
    covered, _ = bp.plan_coverage(bp.discover_chips())
    arm = [b for b, f in covered.items() if bp.fixture_spec(f).target.startswith("thumb")]
    assert arm, "no Cortex-M boards discovered — fixture matching is broken"
    for board in arm:
        assert bp.MODE_BATCH in bp.modes_for(covered[board]), board


def test_a_mode_that_did_not_execute_is_an_error_not_a_number():
    """`ModeNotTakenError` exists and is not silently swallowed as a result."""
    assert issubclass(bp.ModeNotTakenError, Exception)
    # The proof line the CLI prints under --batched, and which measure_once
    # requires before it will believe a batched number.
    proof = "[batched] instructions=200000 batches=391 steps_per_batch=511.51 tick_interval=512"
    m = bp.BATCHED_RE.search(proof)
    assert m and int(m.group(1)) == 200000 and float(m.group(3)) == 511.51


class _Callgrind:
    def __init__(self, stderr: str):
        self.stderr = stderr
        self.returncode = 0


def _fake_callgrind(stderr: str):
    seen: dict = {}

    def run(argv, **kwargs):
        seen["argv"] = argv
        seen["env"] = kwargs.get("env") or {}
        return _Callgrind(stderr)

    return seen, run


_IREFS = "==1== I refs: 1,000\n"
_BATCHED_PROOF = (
    "[batched] instructions=200000 batches=391 "
    "steps_per_batch=511.51 tick_interval=512\n"
)


def test_step_mode_selects_the_arm_step_loop(monkeypatch):
    """`--batched` does not select the ARM loop. The step column has to.

    `run_firmware_arm` takes `run_arm_batched_loop` unless
    `LABWIRED_ARM_SINGLE_STEP=1`. A step measurement that forgets the variable
    records the batched loop a second time, which is how every ARM board came
    to carry the same number in both columns.
    """
    seen, run = _fake_callgrind(_IREFS)
    monkeypatch.setattr(bp.subprocess, "run", run)
    measured = bp.measure_once(
        Path("labwired"), Path("stm32f103.yaml"), Path("fw.elf"), 200_000, bp.MODE_STEP
    )
    assert seen["env"].get("LABWIRED_ARM_SINGLE_STEP") == "1"
    assert "--batched" not in seen["argv"]
    assert measured.irefs == 1000


def test_step_mode_rejects_a_run_that_took_the_batched_loop(monkeypatch):
    """A `[batched]` line in a step run means the lever did not take."""
    _, run = _fake_callgrind(_IREFS + _BATCHED_PROOF)
    monkeypatch.setattr(bp.subprocess, "run", run)
    with pytest.raises(bp.ModeNotTakenError, match="STEP"):
        bp.measure_once(
            Path("labwired"), Path("stm32f103.yaml"), Path("fw.elf"), 200_000, bp.MODE_STEP
        )


def test_batch_mode_still_requires_the_batched_proof_line(monkeypatch):
    """The step lever must not leak into the batch column."""
    seen, run = _fake_callgrind(_IREFS + _BATCHED_PROOF)
    monkeypatch.setattr(bp.subprocess, "run", run)
    bp.measure_once(
        Path("labwired"), Path("stm32f103.yaml"), Path("fw.elf"), 200_000, bp.MODE_BATCH
    )
    assert seen["env"].get("LABWIRED_ARM_SINGLE_STEP") != "1"
    assert "--batched" in seen["argv"]


def test_a_pair_that_measured_one_loop_is_rejected():
    """Near-equal step and batch numbers are the duplicated-loop fingerprint.

    The ARM boards really carried step and batch within a percent of each
    other, both the batched loop.
    An honest pair sits much further apart (a 1100-vs-450 pair is about
    2.4x). A width-1 batch still prints `steps_per_batch=1`, which is a
    different check. 1.25x is below every honest pair on this tree and above
    every duplicated one.
    """
    assert bp.same_loop_baselines({"stm32f103": {"step": 54.0, "batch": 54.0}}) == [
        "stm32f103"
    ]
    assert bp.same_loop_baselines({"esp32": {"step": 1100.2, "batch": 450.6}}) == []
    # One mode only: nothing to compare, so not a duplicate.
    assert bp.same_loop_baselines({"esp32c3": {"batch": 274.4}}) == []


def test_committed_baselines_are_two_different_loops():
    """The gate over the file, not only over a fixture built in the test."""
    baselines = json.loads(bp.BASELINE_PATH.read_text())
    dup = bp.same_loop_baselines(baselines)
    assert not dup, (
        "step and batch baselines are within "
        f"{bp.SAME_LOOP_RATIO}x, so one column measured the other loop: {dup}"
    )


def test_no_baseline_for_a_board_that_is_gone():
    """A stale entry for a deleted chip makes coverage look wider than it is."""
    import json

    covered, _ = bp.plan_coverage(bp.discover_chips())
    baselines = json.loads(bp.BASELINE_PATH.read_text())
    orphans = sorted(set(baselines) - set(covered))
    assert not orphans, f"baselines for chips no longer covered: {orphans}"


# ── Measured vs declared ─────────────────────────────────────────────────────
#
# A fixture MATCH is a build recipe; a BASELINE is the residue of a run that
# actually happened. Conflating the two is how this gate came to report
# "covered: 25 chips across 7 memory maps; 0 waived" while three of those chips
# had never produced a number anywhere — the Xtensa fixture crate has never been
# compiled by CI or by anyone. These tests pin the separation so the count
# cannot quietly re-merge.

_STM32 = {"stm32f103": "stm32"}


def test_never_measured_is_derived_from_the_baseline_file_not_a_list():
    """The third state comes from state, like coverage does.

    A hand-kept "not really measured" list is the same defect one level up: it
    stops being true the moment someone adds a fixture and forgets to edit it,
    which is exactly what happened when WAIVED was emptied.
    """
    covered = {"a": "stm32", "b": "stm32"}
    never = bp.never_measured_board_modes(covered, {"a": {"step": 1.0, "batch": 2.0}})
    assert never == [("b", "step"), ("b", "batch")]
    # And it flips purely on the file's contents — nothing is hard-coded.
    assert bp.never_measured_board_modes(
        covered, {"a": {"step": 1.0, "batch": 2.0}, "b": {"step": 1.0, "batch": 2.0}}
    ) == []


def test_a_baseline_in_one_mode_does_not_cover_the_other_mode():
    """The unit is the board-mode, not the board.

    stm32l476 with a `step` baseline and no `batch` one is not "measured": the
    batched loop the browser runs is the half that would regress unseen, which
    is the whole reason #830's 9-16x win showed up here as +0.2%.
    """
    never = bp.never_measured_board_modes(_STM32, {"stm32f103": {"step": 1503.8}})
    assert never == [("stm32f103", "batch")]
    assert bp.has_been_measured("stm32f103", "step", {"stm32f103": {"step": 1503.8}})
    assert not bp.has_been_measured("stm32f103", "batch", {"stm32f103": {"step": 1503.8}})


def test_measuring_a_board_mode_here_clears_it_from_never_measured():
    """"Never measured ANYWHERE" must include this run, or it lies the other way.

    The state is "no number exists", not "no number was on file when we
    started": a first-ever measurement is still a measurement.
    """
    assert bp.never_measured_board_modes(_STM32, {}) == [
        ("stm32f103", "step"),
        ("stm32f103", "batch"),
    ]
    assert bp.never_measured_board_modes(
        _STM32, {}, measured={"stm32f103": {"step": 1503.8}}
    ) == [("stm32f103", "batch")]


def test_never_measured_note_names_the_toolchain_that_is_missing():
    """The reason is derived from the fixture recipe, so it cannot go stale."""
    note = bp.never_measured_note("esp32", bp.MODE_STEP, "esp32")
    assert "esp" in note and "xtensa-esp32-none-elf" in note


def test_the_xtensa_parts_have_never_been_measured_anywhere():
    """The concrete case, pinned against the real tree.

    It WAS three — esp32[step], esp32s3[step], esp32s3-zero[step] — then one,
    esp32[batch], when classic ESP32 gained `ALL_MODES`. It is now ZERO: every
    board-mode the fixtures match has produced a number.

    So this reads as an emptiness assertion rather than a list. That is the
    goal state, and a test that could only express the un-finished state would
    have to be edited on the day the work completed — which is exactly what
    happened to the "chips with no measured mode at all" assertion below.
    """
    covered, _ = bp.plan_coverage(bp.discover_chips())
    baselines = bp.load_baselines()
    never = bp.never_measured_board_modes(covered, baselines)
    # THE GOAL STATE: nothing is unmeasured. 57 of 57 board-modes, 30 of 30
    # chips. This assertion is the one that says so if that stops being true.
    assert never == [], never
    # And whatever IS listed, if the set ever refills, must genuinely lack a
    # baseline for that MODE — not merely for the board, which was the older
    # and weaker form.
    for board, mode in never:
        assert mode not in baselines.get(board, {}), (board, mode)


def test_matched_board_modes_are_the_union_of_each_fixtures_modes():
    """The denominator every report divides by, stated once."""
    covered, _ = bp.plan_coverage(bp.discover_chips())
    matched = bp.board_modes(covered)
    assert len(matched) == sum(len(bp.modes_for(f)) for f in covered.values())
    assert len(matched) > len(covered), "at least one board must have two modes"
    assert len(set(matched)) == len(matched), "a board-mode must not be counted twice"
    # Board-alphabetical, so the summary and the measurement table read as one
    # list rather than two orderings of the same facts.
    assert [b for b, _ in matched] == sorted(b for b, _ in matched)


def _check_coverage_output() -> str:
    """`--check-coverage` exactly as the PR gate runs it."""
    proc = subprocess.run(
        [sys.executable, str(Path(bp.__file__)), "--check-coverage"],
        capture_output=True,
        text=True,
    )
    assert proc.returncode == 0, proc.stderr
    return proc.stdout


def _coverage_with_a_hole(monkeypatch, capsys, drop=("esp32", "batch")) -> str:
    """`--check-coverage` output with ONE board-mode's baseline removed.

    The never-measured set is EMPTY in this tree now — 57 of 57 board-modes
    carry a baseline. Tests of the never-measured ACCOUNTING therefore cannot
    read a hole off production data any more; two of them carried an explicit
    "this test is vacuous unless something is unmeasured" guard, and those
    guards correctly fired the moment the last hole was filled.

    So the hole is CONSTRUCTED. That is strictly better than depending on the
    tree to contain one: the accounting stays under test however complete the
    real coverage gets, and nobody is ever tempted to leave a board unmeasured
    to keep a test meaningful.

    In-process rather than the `_check_coverage_output` subprocess, because a
    monkeypatched `load_baselines` cannot cross a process boundary and
    `BASELINE_PATH` has no override — adding one to the production script just
    to test it would be the tail wagging the dog.
    """
    real = bp.load_baselines()
    holed = {
        b: {m: v for m, v in modes.items() if (b, m) != drop}
        for b, modes in real.items()
    }
    holed = {b: m for b, m in holed.items() if m}
    monkeypatch.setattr(bp, "load_baselines", lambda: holed)
    monkeypatch.setattr(sys, "argv", ["board_perf.py", "--check-coverage"])
    assert bp.main() == 0
    return capsys.readouterr().out


def test_check_coverage_headline_does_not_count_never_measured_as_coverage(
    monkeypatch, capsys
):
    """The PR gate's headline: first number honest, matched total labelled.

    This is the line that was quoted into a merged PR description as though it
    meant 25 chips were being measured. The property is not "the wording is
    nice" — it is that the first number a skimmer reads is a count of
    board-modes that have actually produced a number.
    """
    real_never = bp.never_measured_board_modes(
        bp.plan_coverage(bp.discover_chips())[0], bp.load_baselines()
    )
    assert real_never == [], (
        "the tree now has a hole again; this test constructs its own, so "
        "update the constructed one only if the real gap is intentional: "
        f"{real_never}"
    )

    out = _coverage_with_a_hole(monkeypatch, capsys)
    headline = out.splitlines()[0]

    covered, _ = bp.plan_coverage(bp.discover_chips())
    matched = bp.board_modes(covered)

    first = int(re.search(r"\d+", headline).group())
    # Exactly one board-mode was removed, so the honest count is one short of
    # the matched total — and the vacuity guard now holds by CONSTRUCTION
    # rather than by the tree happening to be incomplete.
    assert first == len(matched) - 1, headline
    assert first < len(matched), "the constructed hole did not take"
    # The matched total may appear on the headline, but never on its own as the
    # subject of "covers": it is always the denominator of the honest number.
    assert not re.search(rf"covers\s+{len(matched)}\b", headline), headline
    assert not re.search(rf"covers\s+{len(covered)}\s+chips", headline), headline
    assert "NEVER measured anywhere" in headline


def test_check_coverage_names_every_never_measured_board_mode():
    """A number nobody can act on is only half the fix; name them."""
    out = _check_coverage_output()
    covered, _ = bp.plan_coverage(bp.discover_chips())
    never = bp.never_measured_board_modes(covered, bp.load_baselines())
    for board, mode in never:
        assert f"{board}[{mode}]" in out, f"{board}[{mode}] not named in --check-coverage"

    # The stronger statement — a chip with NO measured mode at all — is
    # asserted CONDITIONALLY, because the goal state is that there are none.
    #
    # This used to read `assert "chips with no measured mode at all" in out`
    # unconditionally, and failed the moment the last such chip gained a
    # baseline: a floor that refuses its own goal state. The property worth
    # pinning is "if any exist, they are NAMED", which stays true whether the
    # set is empty or not.
    baselines = bp.load_baselines()
    unmeasured_chips = sorted({b for b, _ in never if not baselines.get(b)})
    if unmeasured_chips:
        assert "chips with no measured mode at all" in out
        for board in unmeasured_chips:
            assert board in out, f"{board} has no measured mode and is not named"
    else:
        assert "chips with no measured mode at all" not in out, (
            "every chip has a measured mode, so this section must not be "
            "printed — an empty heading reads as a finding"
        )


def test_check_coverage_labels_the_matched_total_as_not_a_measurement():
    """The 46/25/7 numbers may be printed — but never as coverage."""
    out = _check_coverage_output()
    assert "a matched fixture is a build recipe, not a measurement" in out


def _stub_a_full_run(monkeypatch, tmp_path, extra_argv=()):
    """Drive main() with the measurement replaced, Xtensa unbuildable as in life.

    valgrind and a release CLI are not available to a unit test, so the two
    expensive halves are stubbed: fixtures "build" for every non-Xtensa memory
    map, and every board-mode measures exactly its own baseline. Measuring the
    baseline back keeps the regression/stale arms quiet, so what the test
    observes is the REPORTING, which is the thing under test.
    """
    covered, _ = bp.plan_coverage(bp.discover_chips())
    xtensa = {f for f in set(covered.values()) if bp.fixture_spec(f).toolchain}
    baselines = bp.load_baselines()

    def fake_build(fixtures):
        built = {f: tmp_path / f for f in fixtures if f not in xtensa}
        for path in built.values():
            path.write_bytes(b"")
        return built, {f: "toolchain for xtensa is not installed" for f in fixtures & xtensa}

    def fake_measure(_cli, board, _firmware, mode):
        interval = bp.MIN_RECOMMENDED_TICK_INTERVAL if mode == bp.MODE_BATCH else None
        value = baselines.get(board, {}).get(mode, 100.0)
        return bp.Measurement(value, 1.0, interval)

    monkeypatch.setattr(bp, "build_fixtures", fake_build)
    monkeypatch.setattr(bp, "measure_board", fake_measure)
    monkeypatch.setattr(bp.shutil, "which", lambda _name: "/usr/bin/valgrind")
    monkeypatch.setattr(
        sys, "argv", ["board_perf.py", "--cli", str(Path(bp.__file__)), *extra_argv]
    )
    return covered, xtensa


def test_run_summary_separates_measured_skipped_and_never_measured(monkeypatch, capsys, tmp_path):
    """The full-run summary, with the Xtensa fixture unbuildable as in real life.

    The three-way split end to end: stm32/nrf/kinetis/rp2xxx/riscv measured, and
    the Xtensa parts reported as never measured rather than folded into a
    "covered" total.
    """
    covered, xtensa = _stub_a_full_run(monkeypatch, tmp_path)

    assert bp.main() == 0
    out = capsys.readouterr().out

    # Every stated total is a measurement, and the matched total is disclaimed.
    measured_bms = len(bp.board_modes({b: f for b, f in covered.items() if f not in xtensa}))
    assert f"measured this run: {measured_bms} board-modes" in out
    assert "matching is not measuring" in out

    # The Xtensa parts appear under NEVER, not under a coverage count.
    # Nothing is never-measured now, so the section must be ABSENT rather
    # than present-and-empty: a heading with no entries reads as a finding.
    assert "NEVER measured anywhere" not in out
    # And "covered: N chips" — the line that overstated — is gone for good.
    assert not re.search(r"^covered:", out, re.M), out


def test_a_baseline_moves_a_board_from_never_measured_to_merely_skipped(
    monkeypatch, capsys, tmp_path
):
    """The middle state, which the tree does not currently exhibit.

    Same run, same absent toolchain — the only thing that changes is that a
    baseline exists. That has to be enough to move the Xtensa parts out of
    "never measured anywhere" and into "skipped here", because it is the whole
    evidence that some machine somewhere did measure them. If this needed a
    list edit as well, the classification would not be derived from state and
    would rot the same way the emptied WAIVED dict did.
    """
    real = bp.load_baselines()
    pretend = {**real, "esp32": {"step": 900.0, "batch": 890.0}}
    monkeypatch.setattr(bp, "load_baselines", lambda: pretend)
    _stub_a_full_run(monkeypatch, tmp_path)

    assert bp.main() == 0
    out = capsys.readouterr().out
    assert "NEVER measured anywhere" not in out
    assert "a baseline from an earlier run is on record" in out
    assert "esp32[batch]" in out


def test_status_json_does_not_call_matched_board_modes_covered(
    monkeypatch, capsys, tmp_path
):
    """The machine-readable twin of the report carries the same split.

    core-perf.yml pastes the report text into an issue body; anything reading
    the JSON instead must not be able to reconstruct the overstatement the text
    no longer makes.
    """
    status = tmp_path / "status.json"
    _stub_a_full_run(monkeypatch, tmp_path, ["--status-json", str(status)])
    assert bp.main() == 0
    capsys.readouterr()

    doc = json.loads(status.read_text())
    assert "covered" not in doc, "a `covered` key is a coverage claim the gate cannot make"
    assert doc["matched"], "the fixture match still has to be reported, just not as coverage"
    assert doc["never_measured"] == [], doc["never_measured"]


# ---------------------------------------------------------------------------
# The committed baselines, read as data.
#
# A mode exists to measure a DIFFERENT loop from its sibling. When a board's
# `step` and `batch` baselines are near-equal, the likeliest explanation is not
# that the two loops cost the same — it is that one mode never selected its
# loop and the harness recorded the other one twice.
#
# That is not hypothetical. `--batched` does not select the ARM loop
# (`run.rs`'s `fast` predicate is already true with no flags), so before
# c7b7a9c8 every ARM board carried `{"batch": 54.0, "step": 54.0}` — bit
# identical — and `run_arm_step_loop` had no perf guard at all. A 10x
# regression in it would have printed 0.0%.
#
# Fixing the harness did not close the hole, because a rebaseline can MISS a
# board: c7b7a9c8 rebaselined 16 of the 29 boards that have a step mode, and
# the nine left behind (a board LIST was written out by hand instead of
# deriving the set) sat at the batched number until #18. Core Perf caught them
# — on push to main, after the merge, because that lane does not gate pull
# requests. This does, in `pr-gate`, in milliseconds.
# ---------------------------------------------------------------------------

#: A mode whose baseline is within this factor of its sibling's is not
#: measuring a different loop. The observed minimum across the fleet is 2.44
#: (esp32: step 1100.2, batch 450.6), so this leaves ~2x headroom before an
#: honest board could trip it. Every stale pair the gap-7 work found sat at
#: ~1.00.
MIN_STEP_BATCH_RATIO = 1.25


def _boards_with_both_modes(node, path=()):
    """Yield `(board, step, batch)` for every board carrying both baselines."""
    if not isinstance(node, dict):
        return
    if any(k in node for k in ("step", "batch")) and all(
        not isinstance(v, dict) for v in node.values()
    ):
        step, batch = node.get("step"), node.get("batch")
        if isinstance(step, (int, float)) and isinstance(batch, (int, float)):
            yield (path[-1] if path else "?"), step, batch
        return
    for key, value in node.items():
        yield from _boards_with_both_modes(value, path + (key,))


def test_no_board_records_the_same_loop_under_both_modes():
    """Reads the committed baselines: two modes must measure two things."""
    data = json.loads(bp.BASELINE_PATH.read_text())
    same = [
        (name, step, batch)
        for name, step, batch in _boards_with_both_modes(data)
        if batch > 0 and step / batch < MIN_STEP_BATCH_RATIO
    ]
    assert not same, (
        "these boards' step and batch baselines are within "
        f"{MIN_STEP_BATCH_RATIO}x of each other, which is what ONE loop "
        "measured twice looks like:\n  "
        + "\n  ".join(f"{n}: step {s}, batch {b} (ratio {s / b:.2f})" for n, s, b in same)
        + "\nEither the mode is not selecting its loop (see the "
        "LABWIRED_ARM_SINGLE_STEP note in board_perf.py), or the baseline is "
        "stale and needs `--update --modes <mode>`. Do not raise "
        "MIN_STEP_BATCH_RATIO to clear this."
    )


def test_the_ratio_gate_catches_a_duplicated_measurement():
    """The half that must not be lost: prove the gate bites.

    A gate over committed data passes for as long as the data is good, which
    is indistinguishable from a gate that cannot fail. So construct the exact
    shape it exists to reject — the `{"batch": 54.0, "step": 54.0}` pair that
    sixteen ARM boards really carried — and assert it is found.
    """
    duplicated = {"stm32f103": {"step": 54.0, "batch": 54.0}}
    found = [
        name
        for name, step, batch in _boards_with_both_modes(duplicated)
        if batch > 0 and step / batch < MIN_STEP_BATCH_RATIO
    ]
    assert found == ["stm32f103"]

    honest = {"esp32": {"step": 1100.2, "batch": 450.6}}
    assert [
        name
        for name, step, batch in _boards_with_both_modes(honest)
        if batch > 0 and step / batch < MIN_STEP_BATCH_RATIO
    ] == []
