from __future__ import annotations

import importlib.util
from pathlib import Path


SCRIPT = Path(__file__).with_name("labwired-audit.py")
SPEC = importlib.util.spec_from_file_location("labwired_audit", SCRIPT)
assert SPEC and SPEC.loader
AUDIT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(AUDIT)


def trace(*pcs: int) -> list[dict[str, int]]:
    return [{"pc": pc} for pc in pcs]


def test_exact_comparison_passes_equal_complete_traces() -> None:
    status, notes, results, scope = AUDIT.compare_traces(
        trace(0x1000, 0x2000), trace(0x1000, 0x2000)
    )

    assert status == "PASS"
    assert len(results) == 2
    assert scope["mode"] == "exact"
    assert "exact scope" in notes


def test_exact_comparison_fails_closed_on_truncated_trace() -> None:
    status, notes, results, scope = AUDIT.compare_traces(
        trace(0x1000, 0x2000), trace(0x1000)
    )

    assert status == "FAIL"
    assert results == []
    assert scope["hw_aligned_steps"] == 2
    assert scope["sim_aligned_steps"] == 1
    assert "Aligned trace lengths differ" in notes


def test_prefix_comparison_requires_explicit_opt_in_and_labels_scope() -> None:
    status, notes, results, scope = AUDIT.compare_traces(
        trace(0x1000, 0x2000), trace(0x1000), allow_prefix=True
    )

    assert status == "PASS"
    assert len(results) == 1
    assert scope["mode"] == "prefix"
    assert "only to the declared common prefix" in notes


def test_bounded_comparison_requires_both_traces_to_supply_requested_steps() -> None:
    status, notes, results, scope = AUDIT.compare_traces(
        trace(0x1000, 0x2000), trace(0x1000), max_steps=2
    )

    assert status == "FAIL"
    assert results == []
    assert scope["mode"] == "bounded"
    assert "requested 2 steps" in notes


def test_pc_drift_fails_with_first_mismatch_recorded() -> None:
    status, notes, results, _scope = AUDIT.compare_traces(
        trace(0x1000, 0x2000), trace(0x1000, 0x3000)
    )

    assert status == "FAIL"
    assert len(results) == 2
    assert results[-1]["match"] is False
    assert "Drift detected at step 1" in notes
