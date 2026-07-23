"""Markdown scoreboard rendering for product matrices."""

from __future__ import annotations

import time
from collections import Counter
from typing import Any


SYM = {
    "pass": "✅",
    "compile_fail": "🔧",
    "compile_timeout": "⏱️",
    "skipped": "⏭️",
    "build_fail": "🔧",
    "build_timeout": "⏱️",
    "toolchain_missing": "📦",
    "pio_missing": "📦",
    "elf_missing": "🔧",
    "boot_fail": "🔴",
    "oracle_fail": "🟠",
    "unmodeled": "🟣",
    "timeout": "⏱️",
    "sim_error": "🔴",
    "skip": "·",
}


def render_scoreboard(
    rows: list[dict[str, Any]],
    *,
    title: str,
    generator: str,
    board_key: str = "board",
    cell_key: str = "sketch",
    board_ids: list[str] | None = None,
    cell_ids: list[str] | None = None,
    legend: str | None = None,
) -> str:
    if board_ids is None:
        board_ids = []
        for r in rows:
            b = r[board_key]
            if b not in board_ids:
                board_ids.append(b)
    if cell_ids is None:
        cell_ids = []
        for r in rows:
            c = r[cell_key]
            if c not in cell_ids:
                cell_ids.append(c)

    by_key = {(r[board_key], r[cell_key]): r for r in rows}
    if legend is None:
        legend = (
            "Legend: ✅ pass · 🔧 compile/build fail · 📦 toolchain missing · "
            "🔴 boot/sim fail · 🟠 oracle miss · 🟣 unmodeled · ⏱️ timeout"
        )

    lines: list[str] = [
        f"# {title}",
        "",
        f"_Generated {time.strftime('%Y-%m-%d %H:%M:%S %z')} by `{generator}`._",
        "",
        legend,
        "",
    ]
    header = "| chip | " + " | ".join(cell_ids) + " | notes |"
    sep = "|------|" + "|".join(["------"] * len(cell_ids)) + "|-------|"
    lines.append(header)
    lines.append(sep)

    for bid in board_ids:
        cells = []
        notes: list[str] = []
        for sid in cell_ids:
            r = by_key.get((bid, sid))
            if not r:
                cells.append("·")
                continue
            cells.append(SYM.get(r["status"], r["status"]))
            if r["status"] != "pass" and r.get("detail"):
                notes.append(f"{sid}:{r['status']}")
        note = "; ".join(notes[:3])
        lines.append(f"| `{bid}` | " + " | ".join(cells) + f" | {note} |")

    lines.append("")
    c = Counter(r["status"] for r in rows)
    total = len(rows)
    lines.append("## Summary")
    lines.append("")
    lines.append(f"- Cells: **{total}**")
    for k, v in sorted(c.items(), key=lambda x: (-x[1], x[0])):
        lines.append(f"- `{k}`: {v}")
    lines.append("")
    lines.append("## Failures (detail)")
    lines.append("")
    for r in rows:
        if r["status"] == "pass":
            continue
        lines.append(f"### `{r[board_key]}` × `{r[cell_key]}` → **{r['status']}**")
        d = r.get("detail")
        if isinstance(d, dict):
            if d.get("oracle"):
                lines.append(str(d["oracle"]))
            if d.get("stderr"):
                lines.append("```")
                lines.append(str(d["stderr"])[-800:])
                lines.append("```")
            if d.get("uart_tail"):
                lines.append("UART tail:")
                lines.append("```")
                lines.append(str(d["uart_tail"]))
                lines.append("```")
        elif d:
            lines.append(str(d)[:500])
        lines.append("")
    return "\n".join(lines) + "\n"
