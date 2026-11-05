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
    "ambiq-apollo4": {
        "crate": "firmware-apollo4p-evb-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/ambiq-apollo4-smoke.yaml",
    },
    "core-v-mcu": {
        "crate": "firmware-cv32e40p-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/core-v-mcu-smoke.yaml",
    },
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
    "mpfs-icicle-kit": {
        "prebuilt_firmware": "tests/fixtures/mpfs-zephyr-hello.elf",
        "script": "examples/renode-onboarding/mpfs-icicle-kit-smoke.yaml",
    },
    "beaglev-fire": {
        "prebuilt_firmware": "tests/fixtures/mpfs-zephyr-hello.elf",
        "script": "examples/renode-onboarding/beaglev-fire-smoke.yaml",
    },
    "starfive-jh7100": {
        "crate": "firmware-starfive-jh7100-smoke",
        "target": "riscv64imac-unknown-none-elf",
        "script": "examples/renode-onboarding/starfive-jh7100-smoke.yaml",
    },
    "ut32m0r500": {
        "crate": "firmware-ut32m0r500-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/ut32m0r500-smoke.yaml",
    },
    "litex_vexriscv_linux": {
        "crate": "firmware-litex-vexriscv-linux-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/litex_vexriscv_linux-smoke.yaml",
    },
    "litex_vexriscv_tftp": {
        "crate": "firmware-litex-vexriscv-tftp-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/litex_vexriscv_tftp-smoke.yaml",
    },
    "litex_vexriscv_verilated_liteuart": {
        "crate": "firmware-litex-vexriscv-smoke",
        "target": "riscv32im-unknown-none-elf",
        "features": "no-unmodelled-spi-probe,verilated-liteuart-profile",
        "script": "examples/renode-onboarding/litex_vexriscv_verilated_liteuart-smoke.yaml",
    },
    "litex_tock": {
        "crate": "firmware-litex-tock-smoke",
        "target": "riscv32imac-unknown-none-elf",
        "script": "examples/renode-onboarding/litex_tock-smoke.yaml",
    },
    "litex_zephyr_vexriscv_i2s": {
        "crate": "firmware-litex-zephyr-i2s-smoke",
        "target": "riscv32imac-unknown-none-elf",
        "script": "examples/renode-onboarding/litex_zephyr_vexriscv_i2s-smoke.yaml",
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
    "eos-s3": {
        "crate": "firmware-eos-s3-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/eos-s3-smoke.yaml",
    },
    "eos-s3-qomu": {
        "crate": "firmware-eos-s3-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/eos-s3-qomu-smoke.yaml",
    },
    "eos-s3-quickfeather": {
        "crate": "firmware-eos-s3-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/eos-s3-quickfeather-smoke.yaml",
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
    "renesas-r7fa2l1a": {
        "crate": "firmware-renesas-r7fa2l1a-smoke",
        "target": "thumbv8m.base-none-eabi",
        "script": "examples/renode-onboarding/renesas-r7fa2l1a-smoke.yaml",
    },
    "renesas-r7fa2e1a9": {
        "crate": "firmware-renesas-r7fa2e1a9-smoke",
        "target": "thumbv8m.base-none-eabi",
        "script": "examples/renode-onboarding/renesas-r7fa2e1a9-smoke.yaml",
    },
    "renesas-r7fa8m1a": {
        "crate": "firmware-renesas-r7fa8m1a-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/renesas-r7fa8m1a-smoke.yaml",
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
    "imxrt500": {
        "crate": "firmware-mimxrt595-evk-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/imxrt500-smoke.yaml",
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
    "ezr32hg320": {
        "crate": "firmware-ezr32hg320-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/ezr32hg320-smoke.yaml",
    },
    "ezr32lg330": {
        "crate": "firmware-ezr32-330-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/ezr32lg330-smoke.yaml",
    },
    "ezr32wg330": {
        "crate": "firmware-ezr32-330-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/ezr32wg330-smoke.yaml",
    },
    "slwstk6220a": {
        "crate": "firmware-ezr32-wonder-gecko-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/slwstk6220a-smoke.yaml",
    },
    "efm32zg222": {
        "crate": "firmware-efm32zg222-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/efm32zg222-smoke.yaml",
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
    "brd4116a": {
        "crate": "firmware-efr32xg26-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/brd4116a-smoke.yaml",
    },
    "brd4117a": {
        "crate": "firmware-efr32xg26-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/brd4117a-smoke.yaml",
    },
    "brd4118a": {
        "crate": "firmware-efr32xg26-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/brd4118a-smoke.yaml",
    },
    "brd4120a": {
        "crate": "firmware-efr32xg26-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/brd4120a-smoke.yaml",
    },
    "brd4121a": {
        "crate": "firmware-efr32xg26-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/brd4121a-smoke.yaml",
    },
    "efr32mg1": {
        "crate": "firmware-efr32mg1-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/efr32mg1-smoke.yaml",
    },
    "sltb001a": {
        "crate": "firmware-efr32mg1-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/sltb001a-smoke.yaml",
    },
    "efr32mg12": {
        "crate": "firmware-efr32mg12-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/efr32mg12-smoke.yaml",
    },
    "efr32mg13": {
        "crate": "firmware-efr32mg13-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/efr32mg13-smoke.yaml",
    },
    "sltb004a": {
        "crate": "firmware-efr32mg12-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/sltb004a-smoke.yaml",
    },
    "efr32mg24": {
        "crate": "firmware-efr32mg24-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/efr32mg24-smoke.yaml",
    },
    "brd4186c": {
        "crate": "firmware-efr32mg24-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/brd4186c-smoke.yaml",
    },
    "brd4162a": {
        "crate": "firmware-efr32mg12-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/brd4162a-smoke.yaml",
    },
    "brd4402a": {
        "crate": "firmware-efr32xg22-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/brd4402a-smoke.yaml",
    },
    "efr32xg22": {
        "crate": "firmware-efr32xg22-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/efr32xg22-smoke.yaml",
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
    "stk3600": {
        "crate": "firmware-efm32-stk-m3-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/stk3600-smoke.yaml",
    },
    "stk3700": {
        "crate": "firmware-efm32-stk-m3-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/stk3700-smoke.yaml",
    },
    "stk3200": {
        "crate": "firmware-stk3200-smoke",
        "target": "thumbv6m-none-eabi",
        "script": "examples/renode-onboarding/stk3200-smoke.yaml",
    },
    "stk3800": {
        "crate": "firmware-stk3800-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stk3800-smoke.yaml",
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
    "max32652": {
        "crate": "firmware-max32652-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/max32652-smoke.yaml",
    },
    "max32652-evkit": {
        "crate": "firmware-max32652-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/max32652-evkit-smoke.yaml",
    },
    "nuvoton_npcx9": {
        "crate": "firmware-nuvoton-npcx9-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/nuvoton_npcx9-smoke.yaml",
    },
    "nuvoton_npcx9m6fb_evb": {
        "crate": "firmware-nuvoton-npcx9-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/nuvoton_npcx9m6fb_evb-smoke.yaml",
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
    "stm32h7": {
        "crate": "firmware-stm32h743-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/stm32h7-smoke.yaml",
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
    "mimxrt1064_evk": {
        "crate": "firmware-imxrt1064-demo",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/mimxrt1064_evk-smoke.yaml",
    },
    "sam4s_xplained": {
        "crate": "firmware-sam4s-xplained-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/sam4s_xplained-smoke.yaml",
    },
    "sam4s": {
        "crate": "firmware-sam4s-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/sam4s-smoke.yaml",
    },
    "sam4s16c": {
        "crate": "firmware-sam4s16c-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/sam4s16c-smoke.yaml",
    },
    "sam4s8b": {
        "crate": "firmware-sam4s8b-smoke",
        "target": "thumbv7em-none-eabi",
        "script": "examples/renode-onboarding/sam4s8b-smoke.yaml",
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
    "verilated_ibex": {
        "crate": "firmware-litex-ibex-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/verilated_ibex-smoke.yaml",
    },
    "litex_picorv32": {
        "crate": "firmware-litex-picorv32-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/litex_picorv32-smoke.yaml",
    },
    "litex_minerva": {
        "crate": "firmware-litex-minerva-smoke",
        "target": "riscv32i-unknown-none-elf",
        "script": "examples/renode-onboarding/litex_minerva-smoke.yaml",
    },
    "litex_vexriscv_smp": {
        "crate": "firmware-litex-vexriscv-smp-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/litex_vexriscv_smp-smoke.yaml",
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
    "ri5cy": {
        "crate": "firmware-vegaboard-ri5cy-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/ri5cy-smoke.yaml",
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
    "kendryte_k210": {
        "crate": "firmware-kendryte-k210-smoke",
        "target": "riscv64imac-unknown-none-elf",
        "script": "examples/renode-onboarding/kendryte_k210-smoke.yaml",
    },
    "miv_rv32": {
        "crate": "firmware-miv-rv32-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/miv_rv32-smoke.yaml",
    },
    "miv": {
        "crate": "firmware-miv-rv32-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/miv-smoke.yaml",
    },
    "miv-board": {
        "crate": "firmware-miv-rv32-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/miv-board-smoke.yaml",
    },
    "miv-board-additional-uarts": {
        "crate": "firmware-miv-rv32-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/miv-board-additional-uarts-smoke.yaml",
        "features": "additional-uarts",
    },
    "murax_vexriscv": {
        "crate": "firmware-murax-vexriscv-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/murax_vexriscv-smoke.yaml",
    },
    "murax_vexriscv_verilated_uart": {
        "crate": "firmware-murax-vexriscv-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/murax_vexriscv_verilated_uart-smoke.yaml",
    },
    "riscv_verilated_uartlite": {
        "crate": "firmware-riscv-verilated-uartlite-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/riscv_verilated_uartlite-smoke.yaml",
    },
    "tock_veer_el2_sim": {
        "crate": "firmware-tock-veer-el2-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/tock_veer_el2_sim-smoke.yaml",
    },
    "fomu": {
        "crate": "firmware-fomu-smoke",
        "target": "riscv32im-unknown-none-elf",
        "script": "examples/renode-onboarding/fomu-smoke.yaml",
    },
    "zolertia-firefly": {
        "crate": "firmware-zolertia-firefly-smoke",
        "target": "thumbv7m-none-eabi",
        "script": "examples/renode-onboarding/zolertia-firefly-smoke.yaml",
    },
    "ramn": {
        "crate": "firmware-ramn-smoke",
        "target": "thumbv8m.main-none-eabi",
        "script": "examples/renode-onboarding/ramn-smoke.yaml",
    },
    "picosoc": {
        "crate": "firmware-picosoc-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/picosoc-smoke.yaml",
    },
    "opentitan-earlgrey": {
        "crate": "firmware-opentitan-earlgrey-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/opentitan-earlgrey-smoke.yaml",
    },
    "opentitan-earlgrey-cw310": {
        "crate": "firmware-opentitan-earlgrey-smoke",
        "target": "riscv32imc-unknown-none-elf",
        "script": "examples/renode-onboarding/opentitan-earlgrey-cw310-smoke.yaml",
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
                    "--script",
                    mapping["script"],
                    "--threshold-seconds",
                    "3600",
                ]
            )
            if mapping.get("prebuilt_firmware"):
                command.extend(["--prebuilt-firmware", mapping["prebuilt_firmware"]])
            else:
                command.extend(["--crate", mapping["crate"], "--target", mapping["target"]])
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
