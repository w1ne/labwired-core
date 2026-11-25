#!/usr/bin/env python3
"""Inventory of end-to-end mixed-signal coverage for every system profile.

The report is mechanical on purpose: a profile counts as covered only when an
integration test loads that profile (or one of its `examples/<dir>/analog-*.yaml`
fixtures) and routes a solved `board.analog.<pad>_volts` signal to a board
endpoint. Device-only unit tests, ADC register tests with no netlist, and
`analog_*` helper tests do not count, because they prove the model and not the
board.

Usage (from the repository root):

    scripts/analog_board_coverage.py            # write docs/analog_board_coverage.md
    scripts/analog_board_coverage.py --stdout   # print the report instead

The generated document is the queue for the mixed-signal lane: the first
uncovered profile in the table is the next board to model.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SYSTEMS = ROOT / "configs" / "systems"
TESTS = ROOT / "crates" / "core" / "tests"
OUT = ROOT / "docs" / "analog_board_coverage.md"

PROFILE_REF = re.compile(r"configs/systems/([\w./-]+)\.ya?ml")
EXAMPLE_REF = re.compile(r"examples/([\w./-]+)/analog-[\w.-]+\.ya?ml")
ADC_ROUTE = re.compile(r"adc\.[A-Za-z0-9_]+\.\d+_volts")
ANALOG_ROUTE = "board.analog."


def profiles() -> list[tuple[str, Path]]:
    """Every system manifest, by declared `name:`, in stable path order."""
    found: list[tuple[str, Path]] = []
    for path in sorted(SYSTEMS.rglob("*.yaml")):
        if path.parent == SYSTEMS or path.parent.name in {"stm32-catalog", "onboarding"}:
            found.append((read_name(path) or path.stem, path))
    return found


def read_name(path: Path) -> str | None:
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith("name:"):
            value = line.split(":", 1)[1].strip()
            return value.strip('"').strip("'")
    return None


def read_chip(path: Path) -> Path | None:
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith("chip:"):
            value = line.split(":", 1)[1].strip().strip('"').strip("'")
            return (path.parent / value).resolve()
    return None


def analog_pad_count(chip: Path | None) -> int:
    if chip is None or not chip.is_file():
        return 0
    text = chip.read_text(encoding="utf-8", errors="replace")
    match = re.search(r"^analog_pins:\n((?:[ \t]+.*\n)+)", text, re.MULTILINE)
    if not match:
        return 0
    # Pad labels are not all `PA0`: nRF parts use `P0.02` and friends.
    return len(re.findall(r"^\s+[A-Za-z0-9_.]+\s*:\s*\{", match.group(1), re.MULTILINE))


def resolve_profile_keys() -> tuple[dict[str, Path], list[str]]:
    """Translate manifest references and example directories to manifest paths.

    A test names a manifest either by its path relative to `configs/systems/`
    (`configs/systems/stm32-catalog/stm32f103c4.yaml`) or by bare file stem
    (`configs/systems/nucleo-f103rb.yaml`). Both forms are registered: the
    relative path first so a nested manifest is never shadowed by a same-stem
    file elsewhere, then the stem for the long-standing short form.
    """
    by_key: dict[str, Path] = {}
    names: list[str] = []
    for name, path in profiles():
        relative = path.relative_to(SYSTEMS).with_suffix("").as_posix()
        names.append(name)
        if relative != path.stem:
            by_key[relative] = path
        by_key.setdefault(path.stem, path)
    return by_key, names


def profile_for_example_dir(directory: str, names: list[str]) -> str | None:
    """The profile an `examples/<dir>` deck belongs to.

    Most boards keep the profile name as the directory name. A few add a
    qualifier: `examples/arduino-mega-2560-io/` is the `arduino-mega-2560`
    profile plus two pots, and its own `system.yaml` says so. Resolve to the
    LONGEST profile name the directory extends, never the other way round: a
    shorter directory (`nucleo-f103rb`) must not credit a longer profile
    (`nucleo-f103rb-epaper`), whose own manifest no test loads.
    """
    if directory in names:
        return directory
    candidates = [name for name in names if directory.startswith(name)]
    return max(candidates, key=len) if candidates else None


def evidence() -> dict[Path, str]:
    """Map manifest path -> the test file that proves its analog endpoint.

    Keyed by path, not by declared `name:`: the onboarding tree keeps its own
    `stm32f401cdu6-blackpill.yaml` under the same name as the catalog one, and
    a name-keyed map credited that uncovered board from its sibling's test.
    Example-deck directories resolve through the declared name, first manifest
    in path order winning, because a directory carries no path of its own.
    """
    by_key, names = resolve_profile_keys()
    by_name: dict[str, Path] = {}
    for name, path in profiles():
        by_name.setdefault(name, path)
    by_profile: dict[Path, str] = {}
    for test in sorted(TESTS.rglob("*.rs")):
        text = test.read_text(encoding="utf-8", errors="replace")
        fixtures = EXAMPLE_REF.findall(text)
        # The test must advance a co-simulation session to a boundary. A file
        # that only lists fixture paths (an existence gate) or only calls into
        # the solver proves nothing about a board.
        if "advance_to(" not in text:
            continue
        # The endpoint must be a route the manifest declares: `board.analog.`
        # (ADC channel or digital pad through the pack map) or an explicit
        # `adc.<peripheral>.<channel>_volts` route such as the Mega's ADC8.
        routed = ANALOG_ROUTE in text or ADC_ROUTE.search(text) is not None
        if not routed and not fixtures:
            continue
        rel = test.relative_to(ROOT).as_posix()
        for reference in PROFILE_REF.findall(text):
            path = by_key.get(reference)
            if path:
                by_profile.setdefault(path, rel)
        for directory in fixtures:
            name = profile_for_example_dir(directory, names)
            if name and name in by_name:
                by_profile.setdefault(by_name[name], rel)
    return by_profile


def report() -> str:
    covered_by = evidence()
    rows: list[tuple[str, str, int, str]] = []
    for name, path in profiles():
        chip = read_chip(path)
        rows.append(
            (
                name,
                chip.stem if chip else "-",
                analog_pad_count(chip),
                covered_by.get(path, ""),
            )
        )
    covered = [row for row in rows if row[3]]
    uncovered = [row for row in rows if not row[3]]
    routable = [row for row in uncovered if row[2] > 0]

    lines = [
        "# End-to-end mixed-signal board coverage",
        "",
        "Generated by `scripts/analog_board_coverage.py`. Do not edit by hand; rerun",
        "the script after adding or removing a board test.",
        "",
        "A profile is **covered** only when an integration test advances a",
        "`CosimSession` that loads that profile (or one of its",
        "`examples/<dir>/analog-*.yaml` decks) and reaches an endpoint route the",
        "manifest declares: `board.analog.<pad>_volts`, which the chip's",
        "`analog_pins:` map resolves to an ADC channel or a voltage-thresholded",
        "digital pad, or an explicit `adc.<peripheral>.<channel>_volts` route.",
        "Device-only unit tests, ADC register tests with no netlist, files that only",
        "list fixture paths, and helper tests do not count: they prove the model,",
        "not the board.",
        "",
        f"- Profiles in `configs/systems/`: **{len(rows)}**",
        f"- Covered end to end: **{len(covered)}**",
        f"- Not covered: **{len(uncovered)}**",
        f"- Not covered but with an `analog_pins:` map: **{len(routable)}** (these are the",
        "  boards the generic NTC and diode decks can reach next; the rest need an",
        "  `analog_pins:` map, an ADC model, or a digital endpoint first)",
        "",
        "The `Analog pad map` column counts `analog_pins:` entries in the chip",
        "descriptor. Every such entry in this tree names an ADC peripheral, so the",
        "count is what the generic decks can reach without new routing.",
        "",
        "## Next queue: not covered, `analog_pins:` map already present",
        "",
    ]
    if routable:
        lines += ["| Profile | Chip | Analog pad map |", "| --- | --- | ---: |"]
        lines += [f"| `{name}` | `{chip}` | {pads} |" for name, chip, pads, _ in routable]
    else:
        lines.append("None.")
    lines += [
        "",
        "## Not covered",
        "",
        "| Profile | Chip | Analog pad map |",
        "| --- | --- | ---: |",
    ]
    lines += [f"| `{name}` | `{chip}` | {pads} |" for name, chip, pads, _ in uncovered]
    lines += ["", "## Covered", "", "| Profile | Chip | Evidence |", "| --- | --- | --- |"]
    lines += [
        f"| `{name}` | `{chip}` | `{evidence_file}` |" for name, chip, _, evidence_file in covered
    ]
    lines += [
        "",
        "## How to add a board",
        "",
        "1. Add `examples/<board>/analog-ntc-diode-system.yaml` with the generic Beta NTC",
        "   divider and the Shockley diode load line, routing to `board.analog.<pad>_volts`.",
        "2. Add a test under `crates/core/tests/analog/` that loads the profile, solves",
        "   the netlists, compares each solved node against an independent closed form,",
        "   advances the session to a boundary, and asserts the firmware-visible",
        "   endpoint (ADC data register or input level).",
        "3. Rerun this script and commit the regenerated document.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stdout", action="store_true", help="print instead of writing the doc")
    args = parser.parse_args()
    text = report()
    if args.stdout:
        sys.stdout.write(text)
        return 0
    OUT.write_text(text, encoding="utf-8")
    print(f"wrote {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
