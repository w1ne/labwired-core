#!/usr/bin/env python3
"""Run configured Renode onboarding smokes and record the remaining inventory."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
SMOKE_RUNNER = REPO_ROOT / "scripts" / "onboarding_smoke.sh"
SCOREBOARD_GENERATOR = REPO_ROOT / "scripts" / "generate_onboarding_scoreboard.py"
DESCRIPTOR_ROOT = REPO_ROOT / "configs" / "onboarding"
SMOKE_MAPPINGS = {
    "nrf52840": {
        "crate": "firmware-nrf52840-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/nrf52840-smoke.yaml",
    },
    "stm32f072b_discovery": {
        "crate": "firmware-stm32f072b-disco-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/stm32f072b_discovery-smoke.yaml",
    },
    "stm32f103": {
        "crate": "firmware-nucleo-f103rb-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/stm32f103-smoke.yaml",
    },
    "stm32f4_discovery": {
        "crate": "firmware-f4-discovery-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32f4_discovery-smoke.yaml",
    },
    "stm32f746": {
        "crate": "firmware-stm32f746-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32f746-smoke.yaml",
    },
    "stm32f777": {
        "crate": "firmware-stm32f746-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32f777-smoke.yaml",
    },
    "stm32f401cdu6-blackpill": {
        "crate": "firmware-f401cdu6-blackpill-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32f401cdu6-blackpill-smoke.yaml",
    },
    "stm32l552": {
        "crate": "firmware-l552ze-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/stm32l552-smoke.yaml",
    },
    "stm32h743": {
        "crate": "firmware-stm32h743-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32h743-smoke.yaml",
    },
    "nucleo_f767zi": {
        "crate": "firmware-stm32f746-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/nucleo_f767zi-smoke.yaml",
    },
    "sam4s_xplained": {
        "crate": "firmware-sam4s-xplained-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/sam4s_xplained-smoke.yaml",
    },
    "sam_e70": {
        "crate": "firmware-sam-e70-renode-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/sam_e70-smoke.yaml",
    },
    "litex_vexriscv": {
        "crate": "firmware-litex-vexriscv-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/litex_vexriscv-smoke.yaml",
    },
    "litex_ibex": {
        "crate": "firmware-litex-ibex-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/litex_ibex-smoke.yaml",
    },
    "litex_picorv32": {
        "crate": "firmware-litex-picorv32-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/litex_picorv32-smoke.yaml",
    },
    "arty_litex_vexriscv": {
        "crate": "firmware-arty-litex-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/arty_litex_vexriscv-smoke.yaml",
    },
    "vegaboard_ri5cy": {
        "crate": "firmware-vegaboard-ri5cy-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/vegaboard_ri5cy-smoke.yaml",
    },
    "sifive-fe310": {
        "crate": "firmware-sifive-fe310-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/sifive-fe310-smoke.yaml",
    },
}


def imported_system(descriptor: Path) -> str:
    for line in descriptor.read_text(encoding="utf-8").splitlines():
        key, separator, value = line.partition(":")
        if separator and key.strip() == "system":
            return value.strip().strip("\"'")
    raise ValueError(f"{descriptor} has no system: reference")


def mapped_system(script: Path) -> Path:
    """Resolve the system the smoke script will actually load."""
    in_inputs = False
    for line in script.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not line.startswith(" "):
            in_inputs = line.strip() == "inputs:"
            continue
        if in_inputs and line.strip().startswith("system:"):
            value = line.split(":", 1)[1].strip().strip("\"'")
            return (script.parent / value).resolve()
    raise ValueError(f"{script} has no inputs.system reference")


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

    not_run_reason = (
        "No compatible firmware crate and smoke script are mapped for this imported "
        "descriptor; no firmware run was attempted."
    )
    attempted = 0
    preserved = 0
    for descriptor in descriptors:
        system = imported_system(descriptor)
        system_path = REPO_ROOT / "configs" / system
        if not system_path.is_file():
            raise FileNotFoundError(f"{descriptor}: referenced system does not exist: {system_path}")
        out_dir = args.metrics_root / descriptor.stem
        mapping = SMOKE_MAPPINGS.get(descriptor.stem)
        metrics_path = out_dir / "onboarding-metrics.json"
        if metrics_path.is_file():
            try:
                existing = json.loads(metrics_path.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError):
                existing = {}
            same_system = existing.get("system") == f"configs/{system}"
            if mapping is None and same_system and existing.get("measurement_mode") in {
                "inventory_only",
                "smoke",
            }:
                preserved += 1
                continue

        command = [
            "bash",
            str(SMOKE_RUNNER),
            "--target-id",
            descriptor.stem,
            "--system",
            f"configs/{system}",
            "--out-dir",
            str(out_dir),
        ]
        if mapping is None:
            command.extend(
                ["--inventory-only", "--not-run-reason", not_run_reason]
            )
        else:
            script_path = REPO_ROOT / mapping["script"]
            if mapped_system(script_path) != system_path.resolve():
                raise ValueError(
                    f"{descriptor}: smoke script {script_path} loads "
                    f"{mapped_system(script_path)}, expected imported system {system_path}"
                )
            command.extend(
                [
                    "--crate",
                    mapping["crate"],
                    "--target",
                    mapping["target"],
                    "--script",
                    mapping["script"],
                    "--threshold-seconds",
                    "3600",
                ]
            )
            attempted += 1

        result = subprocess.run(
            command,
            cwd=REPO_ROOT,
            check=False,
        )
        if mapping is None and result.returncode != 0:
            raise SystemExit(f"inventory record failed for {descriptor.stem}: {result.returncode}")
        if mapping is not None and result.returncode not in (0, 1):
            raise SystemExit(f"smoke runner failed unexpectedly for {descriptor.stem}: {result.returncode}")
        if mapping is not None and not metrics_path.is_file():
            raise SystemExit(f"smoke runner did not write metrics for {descriptor.stem}")

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
    summary = json.loads(args.json_out.read_text(encoding="utf-8"))["summary"]
    print(
        f"Recorded {len(descriptors)} imported descriptors; ran {attempted} configured "
        f"smokes and preserved {preserved} existing measurements."
    )
    print(
        f"Booted: {summary['booted']}/{summary['descriptors_present']}; "
        f"boot + smoke pass: {summary['boot_and_smoke_passed']}/"
        f"{summary['descriptors_present']}; not run: {summary['not_run']}; "
        f"fail: {summary['fail']}."
    )
    print(f"Markdown scoreboard: {args.markdown_out}")
    print(f"JSON scoreboard: {args.json_out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
