# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
# SPDX-License-Identifier: MIT
"""Tests for the per-board perf gate's coverage planning.

The measurement itself needs valgrind and a release CLI, so it is not unit
tested here. What is tested is the property that actually failed in practice:
that a chip in `configs/chips/` cannot end up outside the gate without anyone
being told.
"""

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


def test_nothing_is_waived():
    """Every chip in the tree has a fixture. A new waiver needs a reason here."""
    _, waived = bp.plan_coverage(bp.discover_chips())
    assert waived == {}, f"chips fell out of the gate: {waived}"


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


def test_no_baseline_for_a_board_that_is_gone():
    """A stale entry for a deleted chip makes coverage look wider than it is."""
    import json

    covered, _ = bp.plan_coverage(bp.discover_chips())
    baselines = json.loads(bp.BASELINE_PATH.read_text())
    orphans = sorted(set(baselines) - set(covered))
    assert not orphans, f"baselines for chips no longer covered: {orphans}"
