"""Unit cover for the Arduino-matrix fleet verdict.

Every case but the last builds a SYNTHETIC fleet and a synthetic results tree.
Grading the real out-ci here is impossible (it exists only on a runner), and
grading the real boards.yaml for a *verdict* would make these tests a mirror of
today's fleet: they would go green for the wrong reason the moment someone
waived a cell, and they could never express "a column of pure skips must fail",
because the committed fleet is by design not in that state.

The last case does read the real boards.yaml, and it asserts the gate's own
claim about it -- that the committed fleet is gradeable and that an honest run
over it comes back clean -- rather than restating the fleet's contents.

Each negative case is a real exploit of the verdict this replaced: the inline
block in core-arduino-matrix-smoke.yml passed every one of them.
"""

import json
from pathlib import Path

import matrix_verdict as mv

REPO_ROOT = Path(__file__).resolve().parents[2]
REAL_FLEET = REPO_ROOT / "validation/arduino-matrix/boards.yaml"

SKETCHES = ["L0", "L1", "L2"]
FLEET = {"alpha": set(), "beta": {"L2"}}


def write_fleet(root: Path, *, boards, sketches=SKETCHES):
    """boards: list of (id, [skips], external_compile?)."""
    lines = ["boards:"]
    for entry in boards:
        bid, skips = entry[0], entry[1]
        external = entry[2] if len(entry) > 2 else False
        lines.append(f"  - id: {bid}")
        lines.append(f"    chip: {bid}")
        if skips:
            lines.append(f"    sketches_skip: [{', '.join(skips)}]")
        if external:
            lines.append("    external_compile: { lane: elsewhere }")
    lines.append("sketches:")
    for sid in sketches:
        lines.append(f"  - id: {sid}")
        lines.append(f"    dir: sketches/{sid}")
    path = root / "boards.yaml"
    path.write_text("\n".join(lines) + "\n")
    return path


def write_results(root: Path, rows_by_artifact):
    """rows_by_artifact: list of lists of (board, sketch, status)."""
    out = root / "out-ci"
    for i, rows in enumerate(rows_by_artifact):
        d = out / f"artifact-{i}"
        d.mkdir(parents=True)
        (d / "results.json").write_text(
            json.dumps({"rows": [{"board": b, "sketch": s, "status": st} for b, s, st in rows]})
        )
    return out


def to_rows(triples):
    """grade() consumes results.json rows; the cases author them as triples."""
    return [{"board": b, "sketch": s, "status": st} for b, s, st in triples]


def honest_rows():
    rows = []
    for board, skips in FLEET.items():
        for sid in SKETCHES:
            rows.append((board, sid, "skipped" if sid in skips else "pass"))
    return rows


def test_honest_full_fleet_passes():
    assert mv.grade(to_rows(honest_rows()), FLEET, SKETCHES) == []


def test_all_skipped_fails_even_though_nothing_failed(tmp_path):
    """THE hole this gate was written for.

    Every cell reports `skipped`. Nothing is a failure, the cell count is
    complete, and the verdict this replaced printed "full fleet pass".
    """
    rows = [(b, s, "skipped") for b in FLEET for s in SKETCHES]
    errors = mv.grade(to_rows(rows), FLEET, SKETCHES)
    assert any("no cell passed" in e for e in errors), errors


def test_one_board_going_entirely_dark_fails():
    """A per-board floor the global `passes == 0` rule cannot see."""
    rows = [(b, s, "skipped" if b == "alpha" else "pass") for b in FLEET for s in SKETCHES]
    errors = mv.grade(to_rows(rows), FLEET, SKETCHES)
    assert any(e.startswith("alpha: no sketch passed") for e in errors), errors


def test_undeclared_skip_fails():
    rows = [r for r in honest_rows() if r[:2] != ("alpha", "L1")]
    rows.append(("alpha", "L1", "skipped"))
    errors = mv.grade(to_rows(rows), FLEET, SKETCHES)
    assert any("not in that board's sketches_skip" in e for e in errors), errors


def test_declared_skip_is_accepted():
    """beta/L2 is waived in the fleet, so it must not trip the skip rule."""
    errors = mv.grade(to_rows(honest_rows()), FLEET, SKETCHES)
    assert errors == []


def test_missing_board_fails_against_a_derived_count():
    rows = [r for r in honest_rows() if r[0] != "beta"]
    errors = mv.grade(to_rows(rows), FLEET, SKETCHES)
    assert any("expected 6 cells" in e and "beta/L0" in e for e in errors), errors


def test_toolchain_missing_fails():
    rows = [r for r in honest_rows() if r[:2] != ("alpha", "L0")]
    rows.append(("alpha", "L0", "toolchain_missing"))
    errors = mv.grade(to_rows(rows), FLEET, SKETCHES)
    assert any(e == "alpha/L0: toolchain_missing" for e in errors), errors


def test_duplicate_cell_fails():
    rows = honest_rows() + [("alpha", "L0", "pass")]
    errors = mv.grade(to_rows(rows), FLEET, SKETCHES)
    assert any("duplicate cell alpha/L0" in e for e in errors), errors


def test_result_for_a_board_outside_the_fleet_fails():
    rows = honest_rows() + [("gamma", "L0", "pass")]
    errors = mv.grade(to_rows(rows), FLEET, SKETCHES)
    assert any("not in the simulated fleet" in e for e in errors), errors


def test_external_compile_boards_are_not_counted(tmp_path):
    """They are graded by their own lane; counting them makes the floor unmeetable."""
    fleet_path = write_fleet(
        tmp_path, boards=[("alpha", []), ("beta", ["L2"]), ("elsewhere", [], True)]
    )
    skips, sketch_ids = mv.load_fleet(fleet_path)
    assert set(skips) == {"alpha", "beta"}
    assert skips["beta"] == {"L2"}
    assert sketch_ids == SKETCHES


def test_main_reports_no_results_as_an_error(tmp_path, capsys):
    fleet_path = write_fleet(tmp_path, boards=[("alpha", [])])
    (tmp_path / "out-ci").mkdir()
    rc = mv.main(["--results-dir", str(tmp_path / "out-ci"), "--fleet", str(fleet_path)])
    assert rc == 1
    assert "no results.json" in capsys.readouterr().err


def test_main_end_to_end_over_a_synthetic_tree(tmp_path):
    fleet_path = write_fleet(tmp_path, boards=[("alpha", []), ("beta", ["L2"])])
    # One artifact per board, the way download-artifact lays them out.
    out = write_results(
        tmp_path,
        [
            [("alpha", s, "pass") for s in SKETCHES],
            [("beta", s, "skipped" if s == "L2" else "pass") for s in SKETCHES],
        ],
    )
    assert mv.main(["--results-dir", str(out), "--fleet", str(fleet_path)]) == 0


def test_the_committed_fleet_is_gradeable_and_an_honest_run_is_clean():
    """Reads the real boards.yaml -- the gate's claim about it, not its contents.

    Asserts the fleet parses into a non-trivial simulated set, and that a run in
    which every non-waived cell passes grades clean. If someone adds a board or
    a sketch, this keeps working; if the fleet stops being gradeable, it fails.
    """
    skips, sketch_ids = mv.load_fleet(REAL_FLEET)
    assert len(skips) >= 10, f"simulated fleet collapsed to {len(skips)} boards"
    assert len(sketch_ids) >= 3, sketch_ids

    rows = [
        (board, sid, "skipped" if sid in waived else "pass")
        for board, waived in skips.items()
        for sid in sketch_ids
    ]
    assert mv.grade(to_rows(rows), skips, sketch_ids) == []
