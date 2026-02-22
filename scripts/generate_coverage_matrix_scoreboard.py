#!/usr/bin/env python3
"""
Generate a coverage-matrix scoreboard from per-target artifacts.

Expected layout:
  <matrix_root>/<target-id>/result.json
  <matrix_root>/<target-id>/unsupported-audit/metrics.json
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def _load_json(path: Path) -> dict[str, Any] | None:
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text())
    except Exception:
        return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--matrix-root",
        default="out/coverage-matrix",
        help="Root directory containing per-target matrix outputs.",
    )
    parser.add_argument(
        "--markdown-out",
        default="out/coverage-matrix/scoreboard.md",
        help="Path to write markdown scoreboard.",
    )
    parser.add_argument(
        "--json-out",
        default="out/coverage-matrix/scoreboard.json",
        help="Path to write machine-readable scoreboard.",
    )
    args = parser.parse_args()

    matrix_root = Path(args.matrix_root)
    if not matrix_root.exists():
        raise SystemExit(f"matrix root not found: {matrix_root}")

    rows: list[dict[str, Any]] = []
    for target_dir in sorted(p for p in matrix_root.iterdir() if p.is_dir()):
        result = _load_json(target_dir / "result.json")
        metrics = _load_json(target_dir / "unsupported-audit" / "metrics.json")

        status = "missing"
        stop_reason = "n/a"
        instructions = 0
        unsupported = "n/a"
        support_pct = "n/a"

        if result:
            status = str(result.get("status", "unknown"))
            stop_reason = str(result.get("stop_reason", "unknown"))
            instructions = int(result.get("instructions", 0))

        if metrics:
            unsupported_val = metrics.get("unsupported_total", 0)
            support_pct_val = metrics.get("instruction_support_percent", 0)
            unsupported = str(unsupported_val)
            support_pct = f"{float(support_pct_val):.2f}%"

        rows.append(
            {
                "target_id": target_dir.name,
                "status": status,
                "stop_reason": stop_reason,
                "instructions": instructions,
                "unsupported_total": unsupported,
                "instruction_support_percent": support_pct,
                "artifact_path": str(target_dir),
            }
        )

    pass_count = sum(1 for r in rows if r["status"] == "pass")
    fail_count = sum(1 for r in rows if r["status"] == "fail")
    missing_count = sum(1 for r in rows if r["status"] not in {"pass", "fail"})

    markdown_lines = [
        "# Coverage Matrix Scoreboard",
        "",
        f"- targets: `{len(rows)}`",
        f"- pass: `{pass_count}`",
        f"- fail: `{fail_count}`",
        f"- missing: `{missing_count}`",
        "",
        "| Target | Status | Stop Reason | Instructions | Unsupported | Instruction Support | Artifact |",
        "|---|---|---|---:|---:|---:|---|",
    ]
    for row in rows:
        markdown_lines.append(
            "| `{target_id}` | `{status}` | `{stop_reason}` | `{instructions}` | `{unsupported_total}` | `{instruction_support_percent}` | `{artifact_path}` |".format(
                **row
            )
        )
    markdown_lines.append("")

    markdown_out = Path(args.markdown_out)
    markdown_out.parent.mkdir(parents=True, exist_ok=True)
    markdown_out.write_text("\n".join(markdown_lines))

    json_out = Path(args.json_out)
    json_out.parent.mkdir(parents=True, exist_ok=True)
    json_out.write_text(
        json.dumps(
            {
                "targets": rows,
                "summary": {
                    "targets_total": len(rows),
                    "pass": pass_count,
                    "fail": fail_count,
                    "missing": missing_count,
                },
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
