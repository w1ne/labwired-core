#!/usr/bin/env python3
"""Grade an Arduino-matrix fleet run against the fleet definition.

WHY THIS IS A SCRIPT AND NOT A `run:` BLOCK.

The verdict it replaces lived inline in core-arduino-matrix-smoke.yml and could
report success while proving nothing:

  * `skipped` counted as a pass and NOTHING checked that the skip was declared,
    so any cell that reports `skipped` without a waiver bought a green column.
  * There was no minimum-pass floor at all. A run in which every cell reported
    `skipped` printed "arduino matrix gate: full fleet pass".
  * `EXPECTED` was the literal `16 * 9`, so the fleet could shrink in
    boards.yaml and the floor would follow it down with nobody editing a floor.

The monorepo's sibling gate (brd2709a-arduino-matrix.yml) already carried the
`passes == 0` floor this one lacked — two copies of one rule, one incomplete.
This is that rule, once, with a test.

Every threshold is DERIVED from the fleet definition. Nothing here is a literal
that a shrinking fleet can satisfy.

Usage:
    matrix_verdict.py --results-dir validation/arduino-matrix/out-ci \
                      --fleet validation/arduino-matrix/boards.yaml
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import yaml

# Statuses that are not a failure of the board under test. Everything else --
# sim_error, toolchain_missing, elf_missing, a build stage name -- is a failure.
OK_STATUSES = {"pass", "skipped"}


def load_fleet(path: Path) -> tuple[dict[str, set[str]], list[str]]:
    """Return (declared skips by board, sketch ids) for the SIMULATED fleet.

    Boards carrying `external_compile:` are excluded: they are graded by their
    own lane in the monorepo, not by this workflow, and counting them here would
    make the expected-cell floor unsatisfiable.
    """
    cfg = yaml.safe_load(path.read_text()) or {}
    sketch_ids = [s["id"] for s in cfg.get("sketches") or []]
    skips: dict[str, set[str]] = {}
    for board in cfg.get("boards") or []:
        if board.get("external_compile"):
            continue
        skips[board["id"]] = set(board.get("sketches_skip") or [])
    return skips, sketch_ids


def read_rows(results_dir: Path) -> tuple[list[dict], int]:
    """Collect every row from every results.json under results_dir."""
    files = sorted(results_dir.rglob("results.json"))
    rows: list[dict] = []
    for path in files:
        data = json.loads(path.read_text())
        rows.extend(data.get("rows") or [])
    return rows, len(files)


def grade(rows: list[dict], skips: dict[str, set[str]], sketch_ids: list[str]) -> list[str]:
    """Return failure messages. Empty means the fleet result is honest."""
    errors: list[str] = []
    board_ids = set(skips)

    if not board_ids or not sketch_ids:
        return ["fleet definition names no boards or no sketches"]

    expected = len(board_ids) * len(sketch_ids)
    sketch_set = set(sketch_ids)

    seen: dict[tuple[str, str], str] = {}
    for row in rows:
        cell = (row.get("board"), row.get("sketch"))
        status = row.get("status")
        if cell in seen:
            errors.append(
                f"duplicate cell {cell[0]}/{cell[1]} (statuses {seen[cell]}, {status})"
            )
            continue
        seen[cell] = status
        if cell[0] not in board_ids:
            errors.append(f"result for board {cell[0]!r}, which is not in the simulated fleet")
        elif cell[1] not in sketch_set:
            errors.append(f"result for sketch {cell[1]!r}, which is not in the fleet definition")

    ordered = sorted(seen.items(), key=lambda kv: (kv[0][0] or "", kv[0][1] or ""))

    # 1. Nothing may report a status other than pass or a declared skip.
    for cell, status in ordered:
        if status not in OK_STATUSES:
            errors.append(f"{cell[0]}/{cell[1]}: {status}")

    # 2. A skip must be WAIVED in boards.yaml. An undeclared skip is a hole --
    #    a cell nobody decided to stop testing.
    for cell, status in ordered:
        if status == "skipped" and cell[1] not in skips.get(cell[0], set()):
            errors.append(
                f"{cell[0]}/{cell[1]}: skipped but not in that board's sketches_skip -- "
                "waive it in boards.yaml with a reason, or fix the cell"
            )

    # 3. The fleet must be whole, against a count derived from boards.yaml.
    if len(seen) < expected:
        missing = sorted(f"{b}/{s}" for b in board_ids for s in sketch_ids if (b, s) not in seen)
        errors.append(
            f"expected {expected} cells ({len(board_ids)} boards x {len(sketch_ids)} sketches), "
            f"got {len(seen)}; missing: {', '.join(missing[:12])}"
            + (" ..." if len(missing) > 12 else "")
        )

    passes = sum(1 for st in seen.values() if st == "pass")

    # 4. The sibling gate's floor: a column of pure skips proves nothing.
    if passes == 0:
        errors.append("no cell passed -- the column proves nothing")

    # 5. Per board, which the global floor cannot see: one board going entirely
    #    dark is invisible while fifteen others still pass.
    for board in sorted(board_ids):
        reported = [st for c, st in seen.items() if c[0] == board]
        if reported and not any(st == "pass" for st in reported):
            errors.append(f"{board}: no sketch passed -- this board proves nothing")

    return errors


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Grade an Arduino-matrix fleet run.")
    ap.add_argument("--results-dir", required=True)
    ap.add_argument("--fleet", required=True)
    args = ap.parse_args(argv)

    results_dir = Path(args.results_dir)
    skips, sketch_ids = load_fleet(Path(args.fleet))
    rows, files = read_rows(results_dir)

    if files == 0:
        print(f"::error::no results.json under {results_dir}", file=sys.stderr)
        return 1

    errors = grade(rows, skips, sketch_ids)

    expected = len(skips) * len(sketch_ids)
    n_pass = sum(1 for r in rows if r.get("status") == "pass")
    n_skip = sum(1 for r in rows if r.get("status") == "skipped")
    print(
        f"artifacts={files} cells={len(rows)} expected={expected} "
        f"pass={n_pass} skip={n_skip} fail={len(rows) - n_pass - n_skip}"
    )

    if errors:
        for e in errors:
            print(f"::error::{e}", file=sys.stderr)
        return 1

    print(
        f"arduino matrix gate: {n_pass} pass / {n_skip} declared skip / 0 fail "
        f"across {len(skips)} boards"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
