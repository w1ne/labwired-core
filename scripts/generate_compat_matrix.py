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


def _strip_inline_comment(value: str) -> str:
    """Drop a trailing ` # …` comment, but not a `#` inside a quoted scalar."""
    quote = ""
    for i, ch in enumerate(value):
        if quote:
            if ch == quote:
                quote = ""
        elif ch in "\"'":
            quote = ch
        elif ch == "#" and (i == 0 or value[i - 1] in " \t"):
            return value[:i]
    return value


def _scalar(raw: str) -> str:
    return _strip_inline_comment(raw).strip().strip('"').strip("'")


def parse_yaml_fallback(text: str) -> dict:
    """
    Zero-dependency parser for the handful of chip-config fields this script
    reads: top-level scalars, and each peripheral's `id` and `type`.

    It deliberately parses NOTHING else. The previous version also read
    `base_address` and `irq` — neither of which reaches the output — and the
    `irq` branch did a bare int() on the rest of the line, so the day someone
    wrote `irq: 39  # FDCAN1_IT0 …` in stm32h563.yaml this crashed. Parsing a
    field you do not use is all risk and no benefit.
    """
    result: dict = {"peripherals": []}
    current: dict = {}
    in_peripherals = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("#") or not stripped:
            continue
        # Block form `peripherals:` or empty inline `peripherals: []`.
        # Inline empty-list is common on minimal chips (atmega328p); treating it
        # as a scalar left result["peripherals"] as the string "[]" and broke
        # _consumed with AttributeError on str.get.
        if stripped == "peripherals:" or stripped.startswith("peripherals:"):
            rest = stripped[len("peripherals:") :].strip()
            if rest in ("", "[]"):
                in_peripherals = rest == ""  # only enter list mode for block form
                result["peripherals"] = []
                continue
            # Non-empty inline list — fall through to generic scalar (rare).
        if not in_peripherals:
            if ":" in stripped and not stripped.startswith("-"):
                key, _, val = stripped.partition(":")
                val = _scalar(val)
                if val:
                    result[key.strip()] = val
        elif stripped.startswith("- id:"):
            if current:
                result["peripherals"].append(current)
            current = {"id": _scalar(stripped.split(":", 1)[1])}
        elif stripped.startswith("type:") and current:
            current["type"] = _scalar(stripped.split(":", 1)[1])
    if current:
        result["peripherals"].append(current)
    return result


def _consumed(config: dict) -> tuple:
    """The projection this script actually reads. Equivalence is judged on it."""
    per = config.get("peripherals", []) or []
    if not isinstance(per, list):
        per = []
    types = tuple(
        (p.get("type") if isinstance(p, dict) else None) for p in per
    )
    return (
        config.get("name"),
        config.get("arch"),
        types,
    )


def parse_yaml_simple(path: Path) -> dict:
    """
    Parse a chip config, preferring PyYAML.

    The fallback above is load-bearing: the runner that generates this matrix
    has no pip step, so PyYAML is genuinely absent there. That makes it a
    second parser with its own bugs, which is how a stray inline comment took
    CI down while every developer machine — PyYAML installed — stayed green.

    So when PyYAML IS present, run both and fail loudly on disagreement. The
    fork then cannot drift silently: it is checked on every developer machine
    and in every CI job that does have PyYAML.
    """
    text = path.read_text()
    fallback = parse_yaml_fallback(text)
    if yaml is None:
        return fallback

    real = yaml.safe_load(text) or {}
    if _consumed(real) != _consumed(fallback):
        raise SystemExit(
            f"{path.name}: the zero-dependency parser disagrees with PyYAML.\n"
            f"  PyYAML:   {_consumed(real)}\n"
            f"  fallback: {_consumed(fallback)}\n"
            "Fix parse_yaml_fallback — CI runs it without PyYAML and would "
            "otherwise emit a wrong matrix, or crash, with no local warning."
        )
    return real


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
