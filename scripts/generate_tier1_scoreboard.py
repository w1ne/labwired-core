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
        "# Tier-1 validation matrix",
        "",
        "Every cell links the CI run that produced it; no link → `·` unrecorded.",
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
            status, url = cell.get("status", "unrecorded"), cell.get("run_url")
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
    matrix = json.loads(Path(args.matrix).read_text())
    Path(args.out).write_text(render(matrix))
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
