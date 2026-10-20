#!/usr/bin/env python3
"""Keep the external-device catalog in the configuration reference generated.

WHY THIS EXISTS

`docs/configuration_reference.md` lists one row per canonical external-device
`type:` (116 of them). The list is a copy of the kit registry, and the registry
is already exported as JSON by `gen-peripherals-manifest`. A hand-maintained
copy drifts the day someone adds, renames or re-summarises a kit: the manifest
gate stays green because the manifest is generated, while the doc quietly
advertises a model that no longer exists (or omits one that does).

This gate regenerates the table from that manifest and fails when the committed
block differs. The manifest itself is already checked against the registry --
`crates/core/tests/peripheral_kit_gate.rs` re-runs the generator and diffs --
so this adds no second source of truth, only a rendering of the one we have.

The table between the markers keeps the shape the doc already used: `type`,
Bus and the manifest's one-line summary. The manifest does not carry aliases
(they live in `crates/config/src/embedded.rs`), so the alias prose below the
table stays hand-written.

Usage:
    device_catalog_doc.py --check    # the gate
    device_catalog_doc.py --write    # regenerate the block into the doc
"""

from __future__ import annotations

import argparse
import difflib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DOC = ROOT / "docs/configuration_reference.md"
MANIFEST = ROOT / "crates/core/tests/fixtures/peripherals/manifest.json"

START = "<!-- device-catalog:start -->"
END = "<!-- device-catalog:end -->"

HEADER = ["| `type` | Bus | Model |", "|---|---|---|"]

# Render order matches the doc: buses in the order a reader looks for them,
# then type within each group. The Bus spelling is the doc's, not the
# manifest's raw transport token.
BUS_ORDER = ["UART", "I²C", "SPI", "GPIO", "Analog", "CAN"]
BUS_LABEL = {
    "uart": "UART",
    "i2c": "I²C",
    "spi": "SPI",
    "gpio_group": "GPIO",
    "analog": "Analog",
    "can": "CAN",
}


def natural_key(name: str) -> tuple:
    """`74hc32` before `74hc125`, as the hand table ordered numeric families."""
    parts = re.split(r"(\d+)", name)
    return tuple((0, int(p)) if p.isdigit() else (1, p) for p in parts if p)


def load_entries() -> list[dict]:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    entries = data.get("peripherals")
    if not isinstance(entries, list) or not entries:
        raise SystemExit(f"ERROR: {MANIFEST} has no peripherals list")
    for entry in entries:
        missing = {"device_type", "transport", "summary"} - entry.keys()
        if missing:
            raise SystemExit(
                f"ERROR: {MANIFEST} entry {entry!r} lacks {sorted(missing)}; "
                "was the manifest schema changed?"
            )
        if entry["transport"] not in BUS_LABEL:
            raise SystemExit(
                f"ERROR: {MANIFEST} entry `{entry['device_type']}` has unknown "
                f"transport {entry['transport']!r}; teach BUS_LABEL about it"
            )
    return entries


def render_block(entries: list[dict]) -> str:
    def key(entry: dict) -> tuple:
        label = BUS_LABEL[entry["transport"]]
        return (BUS_ORDER.index(label), natural_key(entry["device_type"]))

    lines = [START, *HEADER]
    for entry in sorted(entries, key=key):
        # The hand table dropped the manifest summary's final full stop; keep
        # that house style so regeneration is not a diff on every sentence.
        summary = entry["summary"].rstrip(".").replace("|", r"\|")
        lines.append(f"| `{entry['device_type']}` | {BUS_LABEL[entry['transport']]} | {summary} |")
    lines.append(END)
    return "\n".join(lines)


def current_block(text: str) -> str:
    start = text.find(START)
    end = text.find(END)
    if start < 0 or end < 0 or end < start:
        raise SystemExit(
            f"ERROR: {DOC.relative_to(ROOT)} lacks the {START} / {END} markers; "
            "wrap the catalog table before running --write"
        )
    return text[start : end + len(END)], start, end + len(END)


def write_block(block: str) -> None:
    text = DOC.read_text(encoding="utf-8")
    _, start, end = current_block(text)
    DOC.write_text(text[:start] + block + text[end:], encoding="utf-8")
    print(f"wrote the device catalog block in {DOC.relative_to(ROOT)}")


def check_block(block: str, entries: list[dict]) -> int:
    text = DOC.read_text(encoding="utf-8")
    committed, _, _ = current_block(text)
    if committed == block:
        print(f"device catalog matches {MANIFEST.relative_to(ROOT)} ({len(entries)} types)")
        return 0

    diff = list(
        difflib.unified_diff(
            committed.splitlines(),
            block.splitlines(),
            fromfile=str(DOC.relative_to(ROOT)),
            tofile=str(MANIFEST.relative_to(ROOT)),
            lineterm="",
            n=1,
        )
    )
    shown = diff[:40]
    for line in shown:
        print(line)
    if len(diff) > len(shown):
        print(f"... {len(diff) - len(shown)} more diff lines")
    print(
        "\ndevice catalog is stale. Run "
        "`python3 scripts/ci/device_catalog_doc.py --write` to regenerate it.",
        file=sys.stderr,
    )
    return 1


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Keep the device catalog generated from the manifest.")
    ap.add_argument("--check", action="store_true", help="the gate")
    ap.add_argument("--write", action="store_true", help="regenerate the doc block")
    args = ap.parse_args(argv)

    entries = load_entries()
    block = render_block(entries)

    if args.write:
        write_block(block)
        return 0
    if args.check:
        return check_block(block, entries)

    ap.print_help()
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
