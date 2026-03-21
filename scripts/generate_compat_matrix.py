#!/usr/bin/env python3
"""
Generate a compatibility matrix JSON from chip configs and example smoke tests.

Walks core/configs/chips/*.yaml to enumerate chips and their peripherals,
then checks core/examples/*/io-smoke.yaml to determine which chips have
validated smoke tests. Outputs JSON to stdout.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    # Fallback: minimal YAML parsing for simple chip configs
    yaml = None  # type: ignore[assignment]


def parse_yaml_simple(path: Path) -> dict:
    """Minimal YAML parser for chip configs (avoids PyYAML dependency in CI)."""
    if yaml:
        return yaml.safe_load(path.read_text()) or {}

    # Bare-bones parser: handles flat keys and peripheral lists
    result: dict = {"peripherals": []}
    current_peripheral: dict = {}
    in_peripherals = False
    for line in path.read_text().splitlines():
        stripped = line.strip()
        if stripped.startswith("#") or not stripped:
            continue
        if stripped == "peripherals:":
            in_peripherals = True
            continue
        if not in_peripherals:
            if ":" in stripped and not stripped.startswith("-"):
                key, _, val = stripped.partition(":")
                val = val.strip().strip('"').strip("'")
                if val:
                    result[key.strip()] = val
        else:
            if stripped.startswith("- id:"):
                if current_peripheral:
                    result["peripherals"].append(current_peripheral)
                current_peripheral = {"id": stripped.split(":", 1)[1].strip().strip('"')}
            elif stripped.startswith("type:") and current_peripheral:
                current_peripheral["type"] = stripped.split(":", 1)[1].strip().strip('"')
            elif stripped.startswith("base_address:") and current_peripheral:
                current_peripheral["base_address"] = stripped.split(":", 1)[1].strip()
            elif stripped.startswith("irq:") and current_peripheral:
                current_peripheral["irq"] = int(stripped.split(":", 1)[1].strip())
    if current_peripheral:
        result["peripherals"].append(current_peripheral)
    return result


def find_smoke_tests(examples_dir: Path) -> dict[str, list[str]]:
    """Map chip names to their available smoke test files."""
    smoke_map: dict[str, list[str]] = {}
    for example_dir in sorted(examples_dir.iterdir()):
        if not example_dir.is_dir():
            continue
        system_yaml = example_dir / "system.yaml"
        if not system_yaml.exists():
            continue

        # Extract chip reference from system.yaml
        chip_ref = None
        for line in system_yaml.read_text().splitlines():
            stripped = line.strip()
            if stripped.startswith("chip:") or stripped.startswith("chip_config:"):
                chip_ref = stripped.split(":", 1)[1].strip().strip('"').strip("'")
                # Extract just the chip name from path
                chip_ref = Path(chip_ref).stem
                break

        if not chip_ref:
            continue

        smoke_files = sorted(
            str(f.name) for f in example_dir.glob("*smoke*.yaml")
        )
        if smoke_files:
            smoke_map.setdefault(chip_ref, []).extend(smoke_files)

    return smoke_map


def main() -> int:
    script_dir = Path(__file__).resolve().parent
    core_root = script_dir.parent
    chips_dir = core_root / "configs" / "chips"
    examples_dir = core_root / "examples"

    if not chips_dir.exists():
        print(f"ERROR: chips dir not found: {chips_dir}", file=sys.stderr)
        return 1

    smoke_map = find_smoke_tests(examples_dir) if examples_dir.exists() else {}

    chips = []
    all_peripheral_types: set[str] = set()

    for chip_file in sorted(chips_dir.glob("*.yaml")):
        if chip_file.name.startswith("ci-fixture"):
            continue  # Skip CI test fixtures

        config = parse_yaml_simple(chip_file)
        name = config.get("name", chip_file.stem)
        arch = config.get("arch", "unknown")
        peripherals = config.get("peripherals", [])

        peripheral_types: dict[str, int] = {}
        for p in peripherals:
            ptype = p.get("type", "unknown")
            peripheral_types[ptype] = peripheral_types.get(ptype, 0) + 1
            all_peripheral_types.add(ptype)

        smoke_tests = smoke_map.get(chip_file.stem, [])

        chips.append({
            "id": chip_file.stem,
            "name": name,
            "arch": arch,
            "peripheral_types": peripheral_types,
            "peripheral_count": len(peripherals),
            "has_smoke_test": len(smoke_tests) > 0,
            "smoke_tests": smoke_tests,
        })

    matrix = {
        "generated_by": "generate_compat_matrix.py",
        "chips": chips,
        "all_peripheral_types": sorted(all_peripheral_types),
        "summary": {
            "total_chips": len(chips),
            "chips_with_smoke": sum(1 for c in chips if c["has_smoke_test"]),
            "chips_without_smoke": sum(1 for c in chips if not c["has_smoke_test"]),
            "peripheral_types_count": len(all_peripheral_types),
        },
    }

    json.dump(matrix, sys.stdout, indent=2)
    print()  # trailing newline
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
