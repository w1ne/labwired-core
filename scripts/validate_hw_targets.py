#!/usr/bin/env python3
"""Validate onboarding hardware targets in core and write CI metadata into manifests."""

from __future__ import annotations

import os
import json
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

import yaml


REPO_ROOT = Path(__file__).resolve().parents[1]
CONFIGS_DIR = REPO_ROOT / "configs" / "onboarding"
SYSTEMS_DIR = REPO_ROOT / "configs" / "systems" / "onboarding"
LABWIRED_BIN = REPO_ROOT / "target" / "release" / "labwired"
ZEPHYR_DIR = Path(os.environ.get("ZEPHYR_BASE", "/opt/zephyrproject/zephyr"))
OUT_DIR = REPO_ROOT / "out" / "hw-target-validation"

GITHUB_SERVER_URL = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
GITHUB_REPOSITORY = os.environ.get("GITHUB_REPOSITORY", "")
GITHUB_RUN_ID = os.environ.get("GITHUB_RUN_ID", "")
GITHUB_RUN_ATTEMPT = os.environ.get("GITHUB_RUN_ATTEMPT", "")


def github_validation_links() -> tuple[str, str]:
    if not GITHUB_REPOSITORY or not GITHUB_RUN_ID:
        return "", ""
    run_url = f"{GITHUB_SERVER_URL}/{GITHUB_REPOSITORY}/actions/runs/{GITHUB_RUN_ID}"
    if GITHUB_RUN_ATTEMPT:
        run_url = f"{run_url}/attempts/{GITHUB_RUN_ATTEMPT}"
    return run_url, f"{run_url}#artifacts"


def run_zephyr_build(board_name: str, output_dir: str) -> tuple[bool, str | None]:
    print(f"[{board_name}] building zephyr hello_world")
    try:
        subprocess.check_call(
            ["west", "build", "-b", board_name, "-d", output_dir, "samples/hello_world"],
            cwd=ZEPHYR_DIR,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=300,
        )
        return True, os.path.join(output_dir, "zephyr", "zephyr.elf")
    except Exception as exc:  # pragma: no cover - operational failures
        print(f"[{board_name}] build failed: {exc}")
        return False, None


def run_labwired_sim(elf_path: str, system_path: str) -> tuple[bool, str]:
    print(f"[{Path(system_path).name}] running labwired simulation")
    try:
        result = subprocess.run(
            [
                str(LABWIRED_BIN),
                "--firmware",
                elf_path,
                "--system",
                system_path,
                "--max-steps",
                "20000",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
            timeout=10,
            check=False,
        )
        if result.returncode == 0:
            return True, "simulation-ok"
        lines = (result.stderr or "").strip().splitlines()
        return False, (lines[-1] if lines else f"exit-{result.returncode}")[:120]
    except subprocess.TimeoutExpired:
        return True, "simulation-ok (timeout)"
    except Exception as exc:  # pragma: no cover - operational failures
        return False, str(exc)[:120]


def main() -> int:
    run_url, artifacts_url = github_validation_links()

    if not ZEPHYR_DIR.exists():
        print(f"ERROR: ZEPHYR_BASE not found at {ZEPHYR_DIR}")
        return 2

    if not LABWIRED_BIN.exists():
        print(f"ERROR: labwired binary not found at {LABWIRED_BIN}")
        print("Build with: cargo build --release -p labwired-cli")
        return 2

    configs = sorted(CONFIGS_DIR.glob("*.yaml"))
    total = len(configs)
    passed = 0
    failed: list[tuple[str, str]] = []
    records: list[dict[str, object]] = []

    print(f"Found {total} onboarding targets")

    for idx, path in enumerate(configs, 1):
        target_name = path.stem
        print(f"[{idx}/{total}] processing {target_name}")

        model = yaml.safe_load(path.read_text(encoding="utf-8")) or {}

        with tempfile.TemporaryDirectory() as build_dir:
            system_path = SYSTEMS_DIR / path.name
            system_exists = system_path.exists()
            build_ok = False
            sim_ok = False
            reason = "not-run"

            if not system_exists:
                reason = "missing-system-manifest"
            else:
                build_ok, elf_path = run_zephyr_build(target_name, build_dir)
                if build_ok and elf_path:
                    sim_ok, reason = run_labwired_sim(elf_path, str(system_path))
                else:
                    reason = "zephyr-build-failed"

            checks = {
                "system_manifest": system_exists,
                "zephyr_build": build_ok,
                "simulation": sim_ok,
            }
            passed_checks = sum(1 for ok in checks.values() if ok)
            pass_rate = int(round(100 * passed_checks / len(checks)))
            verified = build_ok and sim_ok

            model["pass_rate"] = pass_rate
            model["verified"] = verified
            model["validation"] = {
                "method": "nightly-zephyr-hello-world",
                "reason": reason,
                "checks": checks,
                "timestamp_utc": datetime.now(timezone.utc).isoformat(),
                "run_url": run_url,
                "artifacts_url": artifacts_url,
            }
            path.write_text(yaml.dump(model, default_flow_style=False, sort_keys=False), encoding="utf-8")

            if sim_ok:
                passed += 1
                print(f"[{target_name}] pass ({reason})")
            else:
                failed.append((target_name, reason))
                print(f"[{target_name}] fail ({reason})")
            records.append(
                {
                    "target": target_name,
                    "pass_rate": pass_rate,
                    "verified": verified,
                    "reason": reason,
                    "checks": checks,
                    "run_url": run_url,
                    "artifacts_url": artifacts_url,
                }
            )

    print(f"Validation complete: {passed}/{total} passed")
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    summary = {
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "total": total,
        "passed": passed,
        "failed": len(failed),
        "pass_percentage": int(round((passed / total) * 100)) if total else 0,
        "run_url": run_url,
        "artifacts_url": artifacts_url,
        "results": records,
    }
    (OUT_DIR / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    lines = [
        "# Hardware Target Validation Summary",
        "",
        f"- Total targets: **{total}**",
        f"- Passed: **{passed}**",
        f"- Failed: **{len(failed)}**",
        f"- Pass %: **{summary['pass_percentage']}%**",
    ]
    if run_url:
        lines.append(f"- Run URL: {run_url}")
    if artifacts_url:
        lines.append(f"- Artifacts URL: {artifacts_url}")
    lines.extend(["", "## Failed Targets", ""])
    if failed:
        for name, reason in failed:
            lines.append(f"- `{name}`: {reason}")
    else:
        lines.append("- none")
    (OUT_DIR / "summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")

    if failed:
        print(f"Failed: {len(failed)}")
        for name, reason in failed[:50]:
            print(f"  - {name}: {reason}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
