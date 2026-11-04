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
    "A2_CV32E40P": {
        "crate": "firmware-cv32e40p-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/A2_CV32E40P-smoke.yaml",
    },
    "riscv_virt": {
        "crate": "firmware-riscv-virt-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/riscv_virt-smoke.yaml",
    },
    "andes_ae350_n25": {
        "crate": "firmware-andes-ae350-n25-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/andes_ae350_n25-smoke.yaml",
    },
    "renesas-ek_ra8m1": {
        "crate": "firmware-ek-ra8m1-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/renesas-ek_ra8m1-smoke.yaml",
    },
    "cc2538": {
        "crate": "firmware-cc2538dk-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/cc2538-smoke.yaml",
    },
    "stm32w108": {
        "crate": "firmware-stm32w108-boot-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/stm32w108-boot-smoke.yaml",
    },
    "renesas-r7fa6m5b": {
        "crate": "firmware-renesas-r7fa6m5b-boot-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/renesas-r7fa6m5b-boot-smoke.yaml",
    },
    "renesas-ek_ra2e1": {
        "crate": "firmware-renesas-ek-ra2e1-smoke",
        "target": "thumbv8m.base-none-eabi",
        "script": "examples/renode-onboarding/renesas-ek_ra2e1-smoke.yaml",
    },
    "renesas-da14592": {
        "crate": "firmware-renesas-da14592-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/renesas-da14592-smoke.yaml",
    },
    "renesas-ck_ra6m5": {
        "crate": "firmware-renesas-ck-ra6m5-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/renesas-ck_ra6m5-smoke.yaml",
    },
    "nxp-s32k388": {
        "crate": "firmware-s32k388-onboarding-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/nxp-s32k388-smoke.yaml",
    },
    "s32k118": {
        "crate": "firmware-s32k118-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/s32k118-smoke.yaml",
    },
    "mimxrt700_evk": {
        "crate": "firmware-mimxrt700-evk-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/mimxrt700_evk-smoke.yaml",
    },
    "mimxrt798s": {
        "crate": "firmware-mimxrt798s-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/mimxrt798s-smoke.yaml",
    },
    "atsamd51g19a": {
        "crate": "firmware-atsamd51-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/atsamd51g19a-smoke.yaml",
    },
    "atsamd21j17d-aft": {
        "crate": "firmware-samd21-j17d-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/atsamd21j17d-aft-smoke.yaml",
    },
    "arduino_uno_r4_minima": {
        "crate": "firmware-ra4m1-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/arduino_uno_r4_minima-smoke.yaml",
    },
    "renesas-r7fa4m1a": {
        "crate": "firmware-ra4m1-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/renesas-r7fa4m1a-smoke.yaml",
    },
    "arduino_nano_33_ble": {
        "crate": "firmware-arduino-nano-33-ble-smoke",
        "target": "thumbv7em-none-eabihf",
        "script": "examples/renode-onboarding/arduino_nano_33_ble-smoke.yaml",
    },
    "efm32g210": {
        "crate": "firmware-efm32g210-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32g210-smoke.yaml",
    },
    "efm32tg840": {
        "crate": "firmware-efm32tg840-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32tg840-smoke.yaml",
    },
    "efm32jg1": {
        "crate": "firmware-efm32jg1-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/efm32jg1-smoke.yaml",
    },
    "efm32jg12": {
        "crate": "firmware-efm32jg12-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32jg12-smoke.yaml",
    },
    "efm32hg350": {
        "crate": "firmware-efm32hg350-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/efm32hg350-smoke.yaml",
    },
    "efm32g222": {
        "crate": "firmware-efm32g222-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32g222-smoke.yaml",
    },
    "efm32g842": {
        "crate": "firmware-efm32g842-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32g842-smoke.yaml",
    },
    "efm32lg942": {
        "crate": "firmware-efm32lg942-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32lg942-smoke.yaml",
    },
    "efm32lg995": {
        "crate": "firmware-efm32lg995-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32lg995-smoke.yaml",
    },
    "efr32mg26": {
        "crate": "firmware-efr32mg26-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/efr32mg26-smoke.yaml",
    },
    "efm32g890": {
        "crate": "firmware-efm32g842-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32g890-smoke.yaml",
    },
    "efm32gg942": {
        "crate": "firmware-efm32g842-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32gg942-smoke.yaml",
    },
    "efm32gg995": {
        "crate": "firmware-efm32g842-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32gg995-smoke.yaml",
    },
    "efm32wg995": {
        "crate": "firmware-efm32wg995-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32wg995-smoke.yaml",
    },
    "efm32pg12": {
        "crate": "firmware-efm32pg12-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/efm32pg12-smoke.yaml",
    },
    "efm32pg1": {
        "crate": "firmware-efm32pg1-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/efm32pg1-smoke.yaml",
    },
    "efm32g232": {
        "crate": "firmware-efm32g232-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/efm32g232-smoke.yaml",
    },
    "nrf52840": {
        "crate": "firmware-nrf52840-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/nrf52840-smoke.yaml",
    },
    "nrf52840dk_nrf52840": {
        "crate": "firmware-nrf52840-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/nrf52840dk_nrf52840-smoke.yaml",
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
    "stm32f042": {
        "crate": "firmware-stm32f042-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/stm32f042-smoke.yaml",
    },
    "stm32f072": {
        "crate": "firmware-stm32f072-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/stm32f072-smoke.yaml",
    },
    "stm32f4_discovery": {
        "crate": "firmware-f4-discovery-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32f4_discovery-smoke.yaml",
    },
    "stm32f4": {
        "crate": "firmware-f4-discovery-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32f4-smoke.yaml",
    },
    "stm32f4_discovery-kit": {
        "crate": "firmware-f4-discovery-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32f4_discovery-kit-smoke.yaml",
    },
    "stm32f746": {
        "crate": "firmware-stm32f746-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32f746-smoke.yaml",
    },
    "stm32f7_discovery-bb": {
        "crate": "firmware-stm32f746-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32f7_discovery-bb-smoke.yaml",
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
    "nucleo_wba52cg": {
        "crate": "firmware-nucleo-wba52cg-demo",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/nucleo_wba52cg-smoke.yaml",
    },
    "stm32wba52": {
        "crate": "firmware-nucleo-wba52cg-demo",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/stm32wba52-smoke.yaml",
    },
    "stm32h743": {
        "crate": "firmware-stm32h743-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32h743-smoke.yaml",
    },
    "stm32h753": {
        "crate": "firmware-stm32h743-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32h753-smoke.yaml",
    },
    "stm32h747-disco": {
        "crate": "firmware-stm32h743-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32h747-disco-smoke.yaml",
    },
    "stm32h747": {
        "crate": "firmware-stm32h743-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32h747-smoke.yaml",
    },
    "nucleo_h753zi": {
        "crate": "firmware-stm32h743-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/nucleo_h753zi-smoke.yaml",
    },
    "stm32l071": {
        "crate": "firmware-stm32l071-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/stm32l071-smoke.yaml",
    },
    "stm32l072": {
        "crate": "firmware-stm32l072-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/stm32l072-smoke.yaml",
    },
    "stm32l151": {
        "crate": "firmware-stm32l151-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/stm32l151-smoke.yaml",
    },
    "stm32f429": {
        "crate": "firmware-stm32f429-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32f429-smoke.yaml",
    },
    "stm32f412": {
        "crate": "firmware-stm32f429-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32f412-smoke.yaml",
    },
    "nucleo_f767zi": {
        "crate": "firmware-stm32f746-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/nucleo_f767zi-smoke.yaml",
    },
    "imxrt1064": {
        "crate": "firmware-imxrt1064-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/imxrt1064-smoke.yaml",
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
    "litex_vexriscv_zephyr": {
        "crate": "firmware-litex-vexriscv-smoke",
        "target": "riscv32im-unknown-none-elf",
        "features": "no-unmodelled-spi-probe",
        "script": "examples/renode-onboarding/litex_vexriscv_zephyr-smoke.yaml",
    },
    "litex_vexriscv_micropython": {
        "crate": "firmware-litex-vexriscv-smoke",
        "target": "riscv32im-unknown-none-elf",
        "features": "no-unmodelled-spi-probe",
        "script": "examples/renode-onboarding/litex_vexriscv_micropython-smoke.yaml",
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
    "sifive-fu540": {
        "crate": "firmware-sifive-fu540-smoke",
        "target": "riscv64imac-unknown-none-elf",
        "script": "examples/renode-onboarding/sifive-fu540-smoke.yaml",
    },
    "sifive-fu740": {
        "crate": "firmware-sifive-fu740-smoke",
        "target": "riscv64imac-unknown-none-elf",
        "script": "examples/renode-onboarding/sifive-fu740-smoke.yaml",
    },
    "miv_rv32": {
        "crate": "firmware-miv-rv32-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/miv_rv32-smoke.yaml",
    },
    "murax_vexriscv": {
        "crate": "firmware-murax-vexriscv-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/murax_vexriscv-smoke.yaml",
    },
    "picosoc": {
        "crate": "firmware-picosoc-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/picosoc-smoke.yaml",
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
            if mapping.get("features"):
                command.extend(["--features", mapping["features"]])
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
