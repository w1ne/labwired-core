#!/usr/bin/env python3
"""
Demo Dry Run Script (Fallback + Optional Live Mode)

Runs the highest-priority pre-demo checks:
1) Model pipeline (fallback from pre-generated YAML or live AI ingestion)
2) IR conversion + Rust codegen
3) Project wiring (asset init + add-peripheral)
4) Simulator smoke run
5) DAP backend smoke (VS Code debug backend)
6) Docker smoke (optional)
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

import yaml


ROOT = Path(__file__).resolve().parents[2]
AI_DIR = ROOT / "ai"
CORE_DIR = ROOT / "core"
OUT_DIR = AI_DIR / "tests" / "demo_dry_run_output"

PREBUILT_YAML = {
    "LM75B": AI_DIR / "tests" / "lm75b_gen.yaml",
    "ADXL345": AI_DIR / "tests" / "adxl345_gen.yaml",
}


def run(cmd: list[str], cwd: Path | None = None) -> None:
    pretty = " ".join(cmd)
    print(f"\n$ {pretty}", flush=True)
    subprocess.run(cmd, cwd=str(cwd) if cwd else None, check=True)


def copy_prebuilt_yaml(device: str, out_yaml: Path) -> None:
    src = PREBUILT_YAML.get(device.upper())
    if src is None or not src.exists():
        raise FileNotFoundError(
            f"No prebuilt YAML available for device '{device}'. "
            f"Supported: {', '.join(PREBUILT_YAML.keys())}"
        )
    shutil.copyfile(src, out_yaml)
    print(f"Copied prebuilt model: {src} -> {out_yaml}", flush=True)


def run_live_ingestion(device: str, datasheet: Path, pages: str, out_yaml: Path) -> None:
    if not datasheet.exists():
        raise FileNotFoundError(f"Datasheet not found: {datasheet}")
    run(
        [
            "python3",
            "-m",
            "labwired_ai.main",
            "ingest-datasheet",
            "--pdf",
            str(datasheet),
            "--pages",
            pages,
            "--name",
            device,
            "--output",
            str(out_yaml),
        ],
        cwd=AI_DIR,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="LabWired demo dry run")
    parser.add_argument(
        "--mode",
        choices=["fallback", "live"],
        default="fallback",
        help="fallback: use prebuilt YAML; live: call LLM ingestion",
    )
    parser.add_argument(
        "--device",
        default="LM75B",
        help="Peripheral device name (default: LM75B)",
    )
    parser.add_argument(
        "--chip",
        default="ci-fixture-cortex-m3-uart1",
        help="Chip descriptor for asset init (default: ci-fixture-cortex-m3-uart1)",
    )
    parser.add_argument(
        "--datasheet",
        default=str(AI_DIR / "tests" / "fixtures" / "lm75b.pdf"),
        help="Datasheet path for --mode live",
    )
    parser.add_argument(
        "--pages",
        default="1-8",
        help="Datasheet page range for --mode live",
    )
    parser.add_argument(
        "--firmware",
        default=str(CORE_DIR / "tests" / "fixtures" / "uart-ok-thumbv7m.elf"),
        help="Firmware ELF for simulator smoke run",
    )
    parser.add_argument(
        "--docker",
        action="store_true",
        help="Also run Docker image build + runtime checks",
    )
    args = parser.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    project_dir = OUT_DIR / "project"
    if project_dir.exists():
        shutil.rmtree(project_dir)

    device = args.device
    out_yaml = OUT_DIR / f"{device.lower()}_generated.yaml"
    out_ir = OUT_DIR / f"{device.lower()}_ir.json"
    out_driver = OUT_DIR / f"{device.lower()}_driver.rs"

    print("=" * 72, flush=True)
    print("LabWired Demo Dry Run", flush=True)
    print("=" * 72, flush=True)
    print(f"mode={args.mode} device={device} chip={args.chip}", flush=True)
    print(f"output={OUT_DIR}", flush=True)

    # 0) Build local binaries used by demo.
    run(["cargo", "build", "-p", "labwired-cli", "-p", "labwired-dap"], cwd=CORE_DIR)

    # 1) Model generation path.
    if args.mode == "fallback":
        copy_prebuilt_yaml(device, out_yaml)
    else:
        run_live_ingestion(device, Path(args.datasheet), args.pages, out_yaml)

    # 2) Convert model -> Strict IR.
    run(
        ["python3", str(AI_DIR / "labwired_ai" / "convert_to_ir.py"), str(out_yaml), str(out_ir)],
        cwd=ROOT,
    )

    # 3) Generate Rust driver.
    run(
        [
            "cargo",
            "run",
            "--quiet",
            "--manifest-path",
            str(CORE_DIR / "crates" / "cli" / "Cargo.toml"),
            "--",
            "asset",
            "codegen",
            "--input",
            str(out_ir),
            "--output",
            str(out_driver),
        ],
        cwd=ROOT,
    )

    # 4) Create project + wire strict IR peripheral.
    run(
        [
            "cargo",
            "run",
            "--quiet",
            "--manifest-path",
            str(CORE_DIR / "crates" / "cli" / "Cargo.toml"),
            "--",
            "asset",
            "init",
            "--output",
            str(project_dir),
            "--chip",
            args.chip,
        ],
        cwd=ROOT,
    )

    system_path = project_dir / "system.yaml"
    system = yaml.safe_load(system_path.read_text())
    chip_file = project_dir / system["chip"]

    run(
        [
            "cargo",
            "run",
            "--quiet",
            "--manifest-path",
            str(CORE_DIR / "crates" / "cli" / "Cargo.toml"),
            "--",
            "asset",
            "add-peripheral",
            "--chip",
            str(chip_file),
            "--id",
            device,
            "--base",
            "0x40001000",
            "--ir-path",
            str(out_ir),
        ],
        cwd=ROOT,
    )

    # 5) Simulator smoke run.
    run(
        [
            "cargo",
            "run",
            "--quiet",
            "-p",
            "labwired-cli",
            "--",
            "--firmware",
            args.firmware,
            "--system",
            str(system_path),
            "--max-steps",
            "50",
            "--json",
        ],
        cwd=CORE_DIR,
    )

    # 6) DAP smoke run (proxy for VS Code debug backend availability).
    run(["cargo", "test", "-p", "labwired-dap", "--test", "e2e"], cwd=CORE_DIR)

    # 7) Optional Docker smoke checks.
    if args.docker:
        tag = "labwired-demo:dryrun"
        run(
            ["docker", "build", "-f", "core/Dockerfile.ci", "-t", tag, "core"],
            cwd=ROOT,
        )
        run(
            [
                "docker",
                "run",
                "--rm",
                "-v",
                f"{CORE_DIR}:/workspace",
                "-w",
                "/workspace",
                tag,
                "--firmware",
                "tests/fixtures/uart-ok-thumbv7m.elf",
                "--system",
                "configs/systems/ci-fixture-uart1.yaml",
                "--max-steps",
                "20",
                "--json",
            ],
            cwd=ROOT,
        )
        run(
            [
                "docker",
                "run",
                "--rm",
                "--entrypoint",
                "/bin/bash",
                tag,
                "-lc",
                "command -v labwired-dap && labwired --version",
            ],
            cwd=ROOT,
        )

    print("\n" + "=" * 72, flush=True)
    print("Demo dry run PASSED", flush=True)
    print("=" * 72, flush=True)
    print(f"System: {system_path}", flush=True)
    print(f"IR:     {out_ir}", flush=True)
    print(f"Driver: {out_driver}", flush=True)
    print(
        f"Run:    labwired --system {system_path} --firmware <your_demo_firmware.elf>",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except subprocess.CalledProcessError as exc:
        print(f"\nFAILED: command exited with code {exc.returncode}: {' '.join(exc.cmd)}")
        sys.exit(exc.returncode)
    except Exception as exc:  # pragma: no cover - fail fast for operator visibility
        print(f"\nFAILED: {exc}")
        sys.exit(1)
