#!/usr/bin/env python3
"""Print Arduino + Zephyr board coverage vs configs/chips (framework fleet)."""

from __future__ import annotations

import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    print("ERROR: pip install pyyaml", file=sys.stderr)
    sys.exit(2)

ROOT = Path(__file__).resolve().parent.parent
CHIPS = ROOT / "configs" / "chips"
ARD = ROOT / "validation" / "arduino-matrix" / "boards.yaml"
ZEP = ROOT / "validation" / "zephyr-matrix" / "boards.yaml"

# Chips that are CI fixtures / variants, not product fleet targets
SKIP_CHIPS = {
    "ci-fixture-cortex-m3-uart1",
    "ci-fixture-riscv",
    "esp32s3-zero",
}


def main() -> int:
    chips = sorted(
        p.stem
        for p in CHIPS.glob("*.yaml")
        if p.stem not in SKIP_CHIPS and not p.stem.startswith(".")
    )
    ard = yaml.safe_load(ARD.read_text()) if ARD.is_file() else {"boards": [], "sketches": []}
    zep = yaml.safe_load(ZEP.read_text()) if ZEP.is_file() else {"boards": []}
    ard_ids = {b["chip"]: b["id"] for b in ard.get("boards") or []}
    # zephyr uses chip field
    zep_ids = {b["chip"]: b["id"] for b in zep.get("boards") or []}
    sketches = [s["id"] for s in ard.get("sketches") or []]

    print("# Framework fleet report")
    print()
    print(f"Arduino sketches: {', '.join(sketches) or '(none)'}")
    print(f"Product chips under configs/chips: {len(chips)}")
    print()
    print(f"{'chip':16} {'arduino':10} {'zephyr':10}")
    print("-" * 40)
    missing_a = missing_z = 0
    for c in chips:
        a = ard_ids.get(c, "—")
        z = zep_ids.get(c, "—")
        if a == "—":
            missing_a += 1
        if z == "—":
            missing_z += 1
        print(f"{c:16} {str(a):10} {str(z):10}")
    print()
    print(f"Arduino board rows: {len(ard_ids)}  (chips missing Arduino: {missing_a})")
    print(f"Zephyr board rows:  {len(zep_ids)}  (chips missing Zephyr: {missing_z})")
    print()
    print("See validation/FRAMEWORK_FLEET.md for levels + peripheral attach story.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
