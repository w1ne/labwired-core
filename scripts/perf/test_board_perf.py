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

    atmega328p is P0 AVR without a firmware-perf-spin-avr crate / linked ELF
    yet; it is waived in board_perf.WAIVED until that fixture exists. Any other
    chip here is a regression that needs a fixture or a new documented reason.
    """
    _, waived = bp.plan_coverage(bp.discover_chips())
    assert waived == {
        "atmega328p": (
            "no perf-spin fixture for AVR8 yet; CPU P0 without linked spin ELF"
        ),
    }, f"unexpected waivers (add fixture or update this allowlist): {waived}"


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

    These three are matched to a fixture and have no baseline in any mode. If
    that ever changes — a baseline lands, or the parts are removed — this test
    is the thing that says so, rather than the count silently absorbing them.
    """
    covered, _ = bp.plan_coverage(bp.discover_chips())
    baselines = bp.load_baselines()
    never = bp.never_measured_board_modes(covered, baselines)
    assert never == [
        ("esp32", bp.MODE_STEP),
        ("esp32s3", bp.MODE_STEP),
        ("esp32s3-zero", bp.MODE_STEP),
    ], never
    # Not one mode of them: no mode of them.
    for board, _mode in never:
        assert baselines.get(board, {}) == {}, board


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


def test_check_coverage_headline_does_not_count_never_measured_as_coverage():
    """The PR gate's headline: first number honest, matched total labelled.

    This is the line that was quoted into a merged PR description as though it
    meant 25 chips were being measured. The property is not "the wording is
    nice" — it is that the first number a skimmer reads is a count of
    board-modes that have actually produced a number.
    """
    out = _check_coverage_output()
    headline = out.splitlines()[0]

    covered, _ = bp.plan_coverage(bp.discover_chips())
    baselines = bp.load_baselines()
    matched = bp.board_modes(covered)
    never = bp.never_measured_board_modes(covered, baselines)

    first = int(re.search(r"\d+", headline).group())
    assert first == len(matched) - len(never), headline
    assert first < len(matched), "this test is vacuous unless something is unmeasured"
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
    # And the stronger statement: a chip with no measured mode at all.
    assert "chips with no measured mode at all" in out


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
        return bp.Measurement(baselines[board][mode], 1.0, 1)

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
    assert "NEVER measured anywhere (3)" in out
    for board in ("esp32", "esp32s3", "esp32s3-zero"):
        assert f"{board}[step]" in out
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
    pretend = {**real, "esp32": {"step": 900.0}, "esp32s3": {"step": 910.0},
               "esp32s3-zero": {"step": 920.0}}
    monkeypatch.setattr(bp, "load_baselines", lambda: pretend)
    _stub_a_full_run(monkeypatch, tmp_path)

    assert bp.main() == 0
    out = capsys.readouterr().out
    assert "NEVER measured anywhere" not in out
    assert "skipped this run (3)" in out
    assert "a baseline from an earlier run is on record" in out
    for board in ("esp32", "esp32s3", "esp32s3-zero"):
        assert f"{board}[step]" in out


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
    assert sorted((e["board"], e["mode"]) for e in doc["never_measured"]) == [
        ("esp32", "step"),
        ("esp32s3", "step"),
        ("esp32s3-zero", "step"),
    ]
