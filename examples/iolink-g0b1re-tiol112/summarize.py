#!/usr/bin/env python3
"""Make a compact, source-pinned summary from g0_customer_timing's raw JSON."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


MODEL_FILES = [
    "configs/chips/stm32g0b1re.yaml",
    "crates/core/src/network/timed_uart.rs",
    "crates/core/src/peripherals/components/tiol112.rs",
    "crates/core/src/peripherals/stm32g0_clock.rs",
    "crates/core/src/peripherals/rcc.rs",
    "crates/core/src/peripherals/uart.rs",
    "crates/core/src/peripherals/uart/timed.rs",
    "crates/core/tests/g0_customer_timing.rs",
]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--elf", type=Path, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--core-commit", required=True,
                        help="Published commit containing the runtime model; harness hash is separate")
    parser.add_argument("--compiler", required=True)
    parser.add_argument("--artifact-url")
    parser.add_argument("--firmware-manifest", type=Path)
    parser.add_argument("--published-source", type=Path)
    parser.add_argument("--test-status", choices=["passed", "failed"], required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    report = json.loads(args.report.read_text())
    firmware_manifest = (
        json.loads(args.firmware_manifest.read_text()) if args.firmware_manifest else None
    )
    published_source = (
        json.loads(args.published_source.read_text()) if args.published_source else None
    )
    elf_sha = digest(args.elf)
    if firmware_manifest:
        assert firmware_manifest["elf_sha256"] == elf_sha, "ELF/manifest mismatch"
        if firmware_manifest["source_commit"] != args.source_commit:
            assert published_source, "source mismatch requires a published-source byte-match record"
            assert published_source["elf_sha256"] == elf_sha, "published-source ELF mismatch"
            assert published_source["published_commit"] == args.source_commit, "published source mismatch"
    phases = {}
    for record in report["transactions"]:
        key = "/".join(
            str(record.get(k, ""))
            for k in ("phase", "direction", "case", "cadence_case")
        ).rstrip("/")
        phases.setdefault(key, []).append(record)
    groups = {}
    for key, records in phases.items():
        groups[key] = {
            "count": len(records),
            "ta_min_us": min(r["ta_us"] for r in records),
            "ta_max_us": max(r["ta_us"] for r in records),
            "reply_byte_counts": sorted({len(r["reply"]) for r in records}),
            "maximum_reply_byte_gap_tbit": max(
                (r.get("byte_gap_max_tbit") or 0) for r in records
            ),
            "maximum_en_to_first_uart_start_us": max(
                r["en_to_first_uart_start_us"] for r in records
            ),
        }
    summary = {
        "schema": "labwired-g0-tiol112-evidence-v1",
        "test_status": args.test_status,
        "profile": report["profile"],
        "firmware": {
            "repository": "https://github.com/w1ne/iolinki",
            "source_commit": args.source_commit,
            "elf_name": args.elf.name,
            "elf_sha256": elf_sha,
            "compiler": args.compiler,
            "artifact_url": args.artifact_url,
            "build_manifest": firmware_manifest,
            "published_source_match": published_source,
        },
        "runtime_model_commit": args.core_commit,
        "core_workspace_base_commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True
        ).strip(),
        "model_source_sha256": {name: digest(root / name) for name in MODEL_FILES},
        "raw_report_sha256": digest(args.report),
        "cpu_hz": report["cpu_hz"],
        "baud": report["baud"],
        "firmware_dll_timing_enforcement": report["firmware_dll_timing_enforcement"],
        "firmware_dll_initial": report.get("firmware_dll_initial"),
        "firmware_dll_final": report.get("firmware_dll_final"),
        "benchmark_read_count": report["benchmark_read_count"],
        "benchmark_write_count": report["benchmark_write_count"],
        "phase_results": groups,
        "actual_application_request_intervals_us": report[
            "actual_application_request_intervals_us"
        ],
        "negative_controls": report["negative_controls"],
        "limits_checked": report["limits_checked"],
        "fidelity_gaps": report["fidelity_gaps"],
        "limits": report["limits"],
        "timing_reference": report["timing_reference"],
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
