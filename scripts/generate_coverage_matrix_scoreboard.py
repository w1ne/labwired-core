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
    parser.add_argument(
        "--required-target",
        action="append",
        default=[],
        help="Target ID required for pass-rate gating. Can be passed multiple times.",
    )
    parser.add_argument(
        "--min-required-pass-rate",
        type=float,
        default=None,
        help="Minimum pass rate (0..1) required across required targets.",
    )
    args = parser.parse_args()

    matrix_root = Path(args.matrix_root)
    if not matrix_root.exists():
        raise SystemExit(f"matrix root not found: {matrix_root}")

    # Discover target outputs by scanning for result.json recursively.
    # This tolerates both flattened and nested artifact download layouts.
    discovered: dict[str, dict[str, Any]] = {}
    for result_path in sorted(matrix_root.rglob("result.json")):
        target_dir = result_path.parent
        target_id = target_dir.name
        if target_id.startswith("coverage-matrix-"):
            target_id = target_id[len("coverage-matrix-") :]

        result = _load_json(result_path)
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

        discovered[target_id] = {
            "target_id": target_id,
            "status": status,
            "stop_reason": stop_reason,
            "instructions": instructions,
            "unsupported_total": unsupported,
            "instruction_support_percent": support_pct,
            "artifact_path": str(target_dir),
        }

    rows: list[dict[str, Any]] = [discovered[k] for k in sorted(discovered.keys())]

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
    required_targets = set(args.required_target)
    required_rows = [r for r in rows if r["target_id"] in required_targets]
    required_present = {r["target_id"] for r in required_rows}
    missing_required = sorted(required_targets - required_present)
    required_pass = sum(1 for r in required_rows if r["status"] == "pass")
    required_total = len(required_rows)
    required_rate = (required_pass / required_total) if required_total else 0.0

    if required_targets:
        markdown_lines.extend(
            [
                "",
                "## Required Target Gate",
                "",
                f"- required targets: `{required_total}/{len(required_targets)}` present",
                f"- required pass: `{required_pass}`",
                f"- required pass rate: `{required_rate * 100.0:.2f}%`",
            ]
        )
        if args.min_required_pass_rate is not None:
            markdown_lines.append(
                f"- required threshold: `{args.min_required_pass_rate * 100.0:.2f}%`"
            )
        if missing_required:
            markdown_lines.append(
                f"- missing required targets: `{', '.join(missing_required)}`"
            )

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
                "required_gate": {
                    "required_targets": sorted(required_targets),
                    "required_present": sorted(required_present),
                    "required_missing": missing_required,
                    "required_pass": required_pass,
                    "required_total": required_total,
                    "required_pass_rate": required_rate,
                    "required_pass_rate_threshold": args.min_required_pass_rate,
                },
            },
            indent=2,
        )
    )

    if args.min_required_pass_rate is not None:
        if missing_required:
            print(
                "ERROR: missing required targets in matrix artifacts: "
                + ", ".join(missing_required)
            )
            return 2
        if required_rate < args.min_required_pass_rate:
            pct = required_rate * 100.0
            threshold_pct = args.min_required_pass_rate * 100.0
            print(
                f"ERROR: required target pass rate {pct:.2f}% below threshold {threshold_pct:.2f}%"
            )
            return 3

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
