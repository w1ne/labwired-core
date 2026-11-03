#!/usr/bin/env python3
"""Record imported Renode descriptors that have no runnable smoke mapping."""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
SMOKE_RUNNER = REPO_ROOT / "scripts" / "onboarding_smoke.sh"
SCOREBOARD_GENERATOR = REPO_ROOT / "scripts" / "generate_onboarding_scoreboard.py"
DESCRIPTOR_ROOT = REPO_ROOT / "configs" / "onboarding"


def imported_system(descriptor: Path) -> str:
    for line in descriptor.read_text(encoding="utf-8").splitlines():
        key, separator, value = line.partition(":")
        if separator and key.strip() == "system":
            return value.strip().strip("\"'")
    raise ValueError(f"{descriptor} has no system: reference")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--metrics-root",
        type=Path,
        default=REPO_ROOT / "docs" / "coverage" / "renode-onboarding" / "metrics",
    )
    parser.add_argument(
        "--markdown-out",
        type=Path,
        default=REPO_ROOT / "docs" / "coverage" / "renode-onboarding-scoreboard.md",
    )
    parser.add_argument(
        "--json-out",
        type=Path,
        default=REPO_ROOT / "docs" / "coverage" / "renode-onboarding-scoreboard.json",
    )
    args = parser.parse_args()

    descriptors = sorted(DESCRIPTOR_ROOT.glob("*.yaml"))
    if not descriptors:
        parser.error(f"no imported descriptors found under {DESCRIPTOR_ROOT}")

    reason = (
        "No platform-specific firmware crate and smoke script are mapped for this imported "
        "descriptor in the existing onboarding workflow; no firmware run was attempted."
    )
    for descriptor in descriptors:
        system = imported_system(descriptor)
        system_path = REPO_ROOT / "configs" / system
        if not system_path.is_file():
            raise FileNotFoundError(f"{descriptor}: referenced system does not exist: {system_path}")
        out_dir = args.metrics_root / descriptor.stem
        result = subprocess.run(
            [
                "bash",
                str(SMOKE_RUNNER),
                "--inventory-only",
                "--target-id",
                descriptor.stem,
                "--system",
                f"configs/{system}",
                "--not-run-reason",
                reason,
                "--out-dir",
                str(out_dir),
            ],
            cwd=REPO_ROOT,
            check=False,
        )
        if result.returncode != 0:
            raise SystemExit(f"inventory record failed for {descriptor.stem}: {result.returncode}")

    result = subprocess.run(
        [
            sys.executable,
            str(SCOREBOARD_GENERATOR),
            "--metrics-root",
            str(args.metrics_root),
            "--descriptor-root",
            str(DESCRIPTOR_ROOT),
            "--markdown-out",
            str(args.markdown_out),
            "--json-out",
            str(args.json_out),
            "--soft-threshold-seconds",
            "3600",
        ],
        cwd=REPO_ROOT,
        check=False,
    )
    if result.returncode != 0:
        raise SystemExit(f"scoreboard generation failed: {result.returncode}")
    print(f"Recorded {len(descriptors)} imported descriptors; no firmware runs were attempted.")
    print(f"Markdown scoreboard: {args.markdown_out}")
    print(f"JSON scoreboard: {args.json_out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
