#!/usr/bin/env python3
"""Aggregate onboarding smoke metrics into markdown/json scoreboard artifacts."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from statistics import median

try:
    import yaml
except ImportError:  # pragma: no cover - main() refuses to run without PyYAML
    yaml = None

REPO_ROOT = Path(__file__).resolve().parents[1]
KNOWN_STUBS_RS = REPO_ROOT / "crates" / "core" / "src" / "bus" / "known_stubs.rs"
# Allowlisted types that `configure_cortex_m` replaces with the real model at
# construction; every other allowlisted type leaves a window that reads zeros.
REPLACED_AT_CONSTRUCTION = {"nvic", "scb"}


def load_metrics(root: Path) -> list[dict]:
    metrics: list[dict] = []
    for path in sorted(root.rglob("onboarding-metrics.json")):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError):
            continue
        try:
            data["_artifact_path"] = str(path.parent.relative_to(root)) or "."
        except ValueError:
            data["_artifact_path"] = str(path.parent)
        metrics.append(data)
    return metrics


def load_zero_answering_types(path: Path = KNOWN_STUBS_RS) -> set[str]:
    try:
        source = path.read_text(encoding="utf-8")
    except OSError:
        return set()
    types = set(re.findall(r'^\s*\(\s*\n\s*"([a-z0-9_]+)",', source, re.M))
    return types - REPLACED_AT_CONSTRUCTION


def zero_answering_windows(system: str | None, stub_types: set[str]) -> list[str] | None:
    """Peripheral ids in the chip a target's system loads that answer reads with zeros.

    None means it could not be measured (no system path, unreadable YAML, or no
    PyYAML); an empty list means the loaded chip names no stubbed window.
    """
    if yaml is None or not system or not stub_types:
        return None
    system_path = REPO_ROOT / system
    try:
        chip_ref = (yaml.safe_load(system_path.read_text(encoding="utf-8")) or {}).get("chip")
        if not chip_ref:
            return None
        chip = yaml.safe_load((system_path.parent / chip_ref).read_text(encoding="utf-8")) or {}
    except (OSError, yaml.YAMLError, AttributeError):
        return None
    return [
        f"{p.get('id')}:{str(p.get('type')).lower()}"
        for p in chip.get("peripherals") or []
        if isinstance(p, dict) and str(p.get("type", "")).lower() in stub_types
    ]


def annotate_zero_answering_windows(metrics: list[dict]) -> None:
    stub_types = load_zero_answering_types()
    for m in metrics:
        m["zero_answering_windows"] = zero_answering_windows(m.get("system"), stub_types)


def summarize(metrics: list[dict], threshold_seconds: int) -> dict:
    def did_run(m: dict) -> bool:
        default = m.get("status") == "pass" or m.get("failure_stage") == "run_smoke"
        return bool(m.get("run_attempted", default))

    def did_boot(m: dict) -> bool:
        value = m.get("booted")
        return value is True if value is not None else m.get("status") == "pass"

    attempted = sum(1 for m in metrics if did_run(m))
    booted = sum(1 for m in metrics if did_boot(m))
    smoke_passed = sum(1 for m in metrics if bool(m.get("smoke_passed", m.get("status") == "pass")))
    not_run = sum(1 for m in metrics if m.get("status") == "not_run" or not did_run(m))
    passing = sum(1 for m in metrics if m.get("status") == "pass")
    failing = sum(1 for m in metrics if m.get("status") == "fail")
    elapsed_values_ms = [
        int(m.get("elapsed_ms", int(m.get("elapsed_seconds", 0)) * 1000))
        for m in metrics
        if did_run(m)
    ]
    median_elapsed_ms = int(median(elapsed_values_ms)) if elapsed_values_ms else 0
    passed_with_zero_windows = sum(
        1 for m in metrics if m.get("status") == "pass" and m.get("zero_answering_windows")
    )
    return {
        "targets_total": len(metrics),
        "run_attempted": attempted,
        "booted": booted,
        "boot_and_smoke_passed": sum(
            1 for m in metrics if did_boot(m) and bool(m.get("smoke_passed", m.get("status") == "pass"))
        ),
        "not_run": not_run,
        "pass": passing,
        "fail": failing,
        "pass_loading_zero_answering_windows": passed_with_zero_windows,
        "median_elapsed_ms": median_elapsed_ms,
        "median_elapsed_seconds": round(median_elapsed_ms / 1000.0, 3),
        "threshold_seconds": threshold_seconds,
        "threshold_met": sum(1 for m in metrics if did_run(m) and bool(m.get("threshold_met"))),
    }


def build_markdown(
    metrics: list[dict], threshold_seconds: int, descriptors_present: int | None
) -> str:
    summary = summarize(metrics, threshold_seconds)
    total = summary["targets_total"]
    denominator = descriptors_present if descriptors_present is not None else total
    median_elapsed_ms = summary["median_elapsed_ms"]
    median_elapsed_seconds = summary["median_elapsed_seconds"]
    threshold_hits = summary["threshold_met"]

    lines = [
        "# Onboarding Smoke Scoreboard",
        "",
        f"- targets_total: `{total}`",
        f"- run_attempted: `{summary['run_attempted']}`",
        f"- booted: `{summary['booted']}/{denominator}`",
        f"- boot_and_smoke_passed: `{summary['boot_and_smoke_passed']}/{denominator}`",
        f"- not_run: `{summary['not_run']}`",
        f"- pass: `{summary['pass']}`",
        f"- fail: `{summary['fail']}`",
        f"- pass_loading_zero_answering_windows: `{summary['pass_loading_zero_answering_windows']}/{summary['pass']}`",
        f"- median_elapsed_ms: `{median_elapsed_ms}`",
        f"- median_elapsed_seconds: `{median_elapsed_seconds}`",
        f"- threshold_seconds: `{threshold_seconds}`",
        f"- threshold_met: `{threshold_hits}/{summary['run_attempted']}`",
        "",
        "> `booted` means a real run produced a result with at least one executed firmware instruction. A smoke pass proves only the assertions in that target's script, not full peripheral coverage. `not_run` means no firmware run was attempted for that descriptor. `Zero Windows` lists the peripherals in the chip the target's system actually loads that are allowlisted stubs (`crates/core/src/bus/known_stubs.rs`) and answer reads with zeros; a pass with zero windows did not exercise those blocks.",
        "",
        "| Target | Status | Run Attempted | Booted | Smoke Passed | Elapsed (ms) | Elapsed (s) | Threshold Met | Zero Windows | Failure Stage | Hint | Signature |",
        "|---|---|---|---|---|---:|---:|---|---|---|---|---|",
    ]
    if descriptors_present is not None:
        lines.insert(2, f"- descriptors_present: `{descriptors_present}`")

    for m in sorted(metrics, key=lambda row: row.get("target_id", "")):
        target = m.get("target_id", "unknown")
        status = m.get("status", "missing")
        default_attempted = status == "pass" or m.get("failure_stage") == "run_smoke"
        run_attempted = bool(m.get("run_attempted", default_attempted))
        booted = m.get("booted")
        if booted is None and status == "pass":
            booted = True
        booted_label = "yes" if booted is True else "no" if booted is False else "unknown"
        if not run_attempted:
            booted_label = "n/a"
        smoke_value = m.get("smoke_passed", status == "pass")
        smoke_label = "n/a" if not run_attempted else "yes" if bool(smoke_value) else "no"
        elapsed_ms = int(m.get("elapsed_ms", int(m.get("elapsed_seconds", 0)) * 1000))
        elapsed_seconds = round(elapsed_ms / 1000.0, 3)
        threshold_value = m.get("threshold_met", False)
        threshold_label = str(bool(threshold_value)) if run_attempted else "n/a"
        windows = m.get("zero_answering_windows")
        windows_label = (
            "n/a" if windows is None else "0" if not windows else f"{len(windows)}: " + ", ".join(f"`{w}`" for w in windows)
        )
        failure_stage = m.get("failure_stage") or "n/a"
        hint = (m.get("failure_hint") or "n/a").replace("|", "/")
        signature = (m.get("first_error_signature") or "n/a").replace("|", "/")
        lines.append(
            f"| `{target}` | `{status}` | `{run_attempted}` | `{booted_label}` | `{smoke_label}` | `{elapsed_ms}` | `{elapsed_seconds}` | `{threshold_label}` | {windows_label} | `{failure_stage}` | {hint} | {signature} |"
        )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--metrics-root", required=True, type=Path)
    parser.add_argument("--markdown-out", required=True, type=Path)
    parser.add_argument("--json-out", required=True, type=Path)
    parser.add_argument("--descriptor-root", type=Path)
    parser.add_argument("--soft-threshold-seconds", type=int, default=3600)
    args = parser.parse_args()
    if yaml is None:
        # Without PyYAML every target's zero-answering windows would be
        # unmeasurable, and the scoreboard would quietly lose the column.
        parser.error("PyYAML is required to resolve zero-answering windows")

    metrics = load_metrics(args.metrics_root)
    annotate_zero_answering_windows(metrics)
    summary = summarize(metrics, args.soft_threshold_seconds)
    descriptors_present = len(list(args.descriptor_root.glob("*.yaml"))) if args.descriptor_root else None
    summary["descriptors_present"] = descriptors_present
    if descriptors_present is not None:
        summary["unmeasured_descriptors"] = max(0, descriptors_present - len(metrics))
    payload = {"summary": summary, "targets": metrics}

    args.markdown_out.parent.mkdir(parents=True, exist_ok=True)
    args.json_out.parent.mkdir(parents=True, exist_ok=True)
    args.markdown_out.write_text(
        build_markdown(metrics, args.soft_threshold_seconds, descriptors_present), encoding="utf-8"
    )
    args.json_out.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
