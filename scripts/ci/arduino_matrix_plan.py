#!/usr/bin/env python3
"""Print the Arduino-matrix job matrix, computed from validation/arduino-matrix/boards/.

core-arduino-matrix-smoke.yml runs this in a first job and feeds the result to
`strategy.matrix` with `fromJSON`, so adding a board is one new file under
boards/ and no workflow edit. The set is every board without `external_compile:`
(those are graded by the lane that owns their toolchain, not by this workflow);
scripts/ci/chip_coverage.py uses the same function, so the gate and the
workflow cannot disagree about which boards run here.

    python3 scripts/ci/arduino_matrix_plan.py >> "$GITHUB_OUTPUT"
    # matrix={"board":["atmega328p","esp32",...]}
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "validation"))
import registry_dir  # noqa: E402

BOARDS_DIR = REPO_ROOT / "validation" / "arduino-matrix" / "boards"


def simulated_board_ids(boards_dir: Path = BOARDS_DIR) -> list[str]:
    """Sorted ids of the boards this repo's workflow simulates."""
    reg = registry_dir.load_yaml_registry(boards_dir, "boards")
    return sorted(b["id"] for b in reg["boards"] if not b.get("external_compile"))


def main() -> int:
    try:
        ids = simulated_board_ids()
    except registry_dir.RegistryError as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1
    if not ids:
        print("::error::the Arduino fleet has no simulated boards", file=sys.stderr)
        return 1
    print("matrix=" + json.dumps({"board": ids}, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
