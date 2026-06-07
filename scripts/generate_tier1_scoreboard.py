#!/usr/bin/env python3
"""Render docs/coverage/tier1-matrix.json as a chip × peripheral markdown grid.

Proof-artifact bar (spec wedge-alignment): a cell renders its real status
ONLY if it carries a run_url; cells without evidence render as unrecorded.
"""
import argparse
import json
from pathlib import Path

ICONS = {"pass": "✅", "partial": "🟡", "blocked": "⛔", "na": "—", "unrecorded": "·"}
RUBRIC = ["clock", "gpio", "uart", "timer", "dma", "irq"]


def render(matrix: dict) -> str:
    # Column set = rubric order, then any extra classes seen (e.g. S3 beachhead).
    extras = sorted({c for row in matrix.values() for c in row if c not in RUBRIC})
    classes = RUBRIC + extras
    lines = [
        "# Tier-1 Validation Matrix",
        "",
        "Every cell links the CI run that produced it; no link → `·` unrecorded.",
        "",
        "**Confidence tier:** ✅ means *sim-consistent* — the check passed against",
        "the simulator's peripheral models on real firmware. Silicon-anchored",
        "verification (hardware-in-the-loop capture replay) is a separate tier",
        "that arrives with the HIL workstream; no cell currently claims it.",
        "",
        "| chip | " + " | ".join(classes) + " |",
        "|---|" + "---|" * len(classes),
    ]
    for chip in sorted(matrix):
        row = matrix[chip]
        cells = []
        for cls in classes:
            cell = row.get(cls)
            if cell is None:
                cells.append("·")
                continue
            status = cell.get("status", "unrecorded")
            url = cell.get("run_url")
            # Malformed evidence demotes to unrecorded.
            if url and (not url.startswith("https://") or any(c in url for c in " |()")):
                url = None
            if status not in ("na", "unrecorded") and not url:
                status = "unrecorded"  # no evidence, no claim
            icon = ICONS.get(status, "·")
            cells.append(f"[{icon}]({url})" if url else icon)
        lines.append(f"| {chip} | " + " | ".join(cells) + " |")
    return "\n".join(lines) + "\n"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--matrix", default="docs/coverage/tier1-matrix.json")
    ap.add_argument("--out", default="docs/coverage/tier1-scoreboard.md")
    args = ap.parse_args()
    path = Path(args.matrix)
    if not path.exists():
        raise SystemExit(f"matrix not found: {path}")
    matrix = json.loads(path.read_text())
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(render(matrix))
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
