#!/usr/bin/env python3
"""Aggregate onboarding smoke metrics into markdown/json scoreboard artifacts."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from statistics import median


def load_metrics(root: Path) -> list[dict]:
    metrics: list[dict] = []
    for path in sorted(root.rglob("onboarding-metrics.json")):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError):
            continue
        data["_artifact_path"] = str(path.parent)
        metrics.append(data)
    return metrics


def build_markdown(metrics: list[dict], threshold_seconds: int) -> str:
    total = len(metrics)
    passing = sum(1 for m in metrics if m.get("status") == "pass")
    failing = total - passing
    elapsed_values = [int(m.get("elapsed_seconds", 0)) for m in metrics]
    median_elapsed = int(median(elapsed_values)) if elapsed_values else 0
    threshold_hits = sum(1 for m in metrics if bool(m.get("threshold_met")))

    lines = [
        "# Onboarding Smoke Scoreboard",
        "",
        f"- targets_total: `{total}`",
        f"- pass: `{passing}`",
        f"- fail: `{failing}`",
        f"- median_elapsed_seconds: `{median_elapsed}`",
        f"- threshold_seconds: `{threshold_seconds}`",
        f"- threshold_met: `{threshold_hits}/{total}`",
        "",
        "| Target | Status | Elapsed (s) | Threshold Met | Failure Stage | Signature |",
        "|---|---|---:|---|---|---|",
    ]

    for m in sorted(metrics, key=lambda row: row.get("target_id", "")):
        target = m.get("target_id", "unknown")
        status = m.get("status", "missing")
        elapsed = m.get("elapsed_seconds", 0)
        threshold_met = m.get("threshold_met", False)
        failure_stage = m.get("failure_stage") or "n/a"
        signature = (m.get("first_error_signature") or "n/a").replace("|", "/")
        lines.append(
            f"| `{target}` | `{status}` | `{elapsed}` | `{threshold_met}` | `{failure_stage}` | `{signature}` |"
        )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--metrics-root", required=True, type=Path)
    parser.add_argument("--markdown-out", required=True, type=Path)
    parser.add_argument("--json-out", required=True, type=Path)
    parser.add_argument("--soft-threshold-seconds", type=int, default=3600)
    args = parser.parse_args()

    metrics = load_metrics(args.metrics_root)
    summary = {
        "targets_total": len(metrics),
        "pass": sum(1 for m in metrics if m.get("status") == "pass"),
        "fail": sum(1 for m in metrics if m.get("status") != "pass"),
        "median_elapsed_seconds": int(median([int(m.get("elapsed_seconds", 0)) for m in metrics]))
        if metrics
        else 0,
        "threshold_seconds": args.soft_threshold_seconds,
        "threshold_met": sum(1 for m in metrics if bool(m.get("threshold_met"))),
    }
    payload = {"summary": summary, "targets": metrics}

    args.markdown_out.parent.mkdir(parents=True, exist_ok=True)
    args.json_out.parent.mkdir(parents=True, exist_ok=True)
    args.markdown_out.write_text(
        build_markdown(metrics, args.soft_threshold_seconds), encoding="utf-8"
    )
    args.json_out.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

