[← Back to Hub](../README.md)

# Top-20 Coverage Matrix

Date: February 26, 2026
Owner: Core simulation team
Goal: Prioritize target bring-up and track deterministic smoke readiness.

## Status Legend

- `green`: deterministic smoke passing with reproducible output.
- `yellow`: runnable, but partial peripheral fidelity or unstable assertions.
- `red`: blocked (missing critical peripheral behavior or reset path).
- `backlog`: not implemented yet.

## Selection Basis

Top-20 is ranked by a weighted mix:

1. Market pull (ecosystem size, expected user demand).
2. Current failure pressure (how often gaps appear in our examples/tests).
3. Architectural leverage (how much one target unlocks related boards/families).

## Top-5 Focus for Current Quarter

| Rank | Target ID | MCU / Board | Why It Is in Top-5 | Current Status |
|---|---|---|---|---|
| 1 | `stm32f103-bluepill` | STM32F103C8 / Blue Pill | Large install base, broad firmware sample availability | `green` |
| 2 | `stm32h563-nucleo` | STM32H563ZI / NUCLEO-H563ZI | Existing flagship demo path and enterprise narrative | `green` |
| 3 | `stm32f401-nucleo` | STM32F401RE / NUCLEO-F401RE | Tier-1 strategy target and common Cortex-M4 baseline | `green` |
| 4 | `firmware-rv32i-ci-fixture` | RV32I CI fixture board | Coverage for non-ARM path and CI smoke reliability | `green` |
| 5 | `firmware-stm32f103-blinky-stm32f103` | STM32F103 demo system | Deterministic integration smoke for common demo path | `green` |

CI alignment note:
- These are the hard-gated targets enforced in `core/.github/workflows/core-coverage-matrix-smoke.yml`.
- `ci-fixture-armv6m` also runs in the matrix as a fast sentinel target, but is not part of the Top-5 hard gate.

## Full Top-20 Matrix

| Rank | Target ID | MCU Family | Representative Board | Arch | SDK / Firmware Fixture | System Manifest | Smoke Script | Tier-1 Peripheral Baseline (`rcc/clock`, `gpio`, `uart`, `timer`, `dma`, `irq`) | Status | Known Gaps |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | `stm32f103-bluepill` | STM32F1 | Blue Pill | ARMv7-M | `firmware-stm32f103-uart` | `core/configs/systems/stm32f103-integrated-test.yaml` | `core/examples/tests/stm32f103_integrated_test.yaml` | `clock/gpio/uart/irq` mostly usable; `timer/dma` partial | `yellow` | Timer/DMA breadth |
| 2 | `stm32h563-nucleo` | STM32H5 | NUCLEO-H563ZI | ARMv8-M-ish firmware path; simulated via current core support subset | `firmware-h563-demo` | `core/examples/nucleo-h563zi/system.yaml` | `core/examples/nucleo-h563zi/uart-smoke.yaml` | Baseline smoke path present; deeper peripherals partial | `yellow` | Broader peripheral fidelity |
| 3 | `stm32f401-nucleo` | STM32F4 | NUCLEO-F401RE | ARMv7E-M | `firmware-f401-demo` | `core/configs/systems/nucleo-f401re.yaml` | `core/examples/nucleo-f401re/uart-smoke.yaml` | `rcc/gpio/uart/systick` deterministic smoke path | `green` | Deep peripheral breadth remains |
| 4 | `firmware-stm32f103-blinky-stm32f103` | STM32F1 | Demo blinky system | ARMv7-M | `firmware-stm32f103-blinky` | `core/examples/firmware-stm32f103-blinky/system.yaml` | `core/examples/firmware-stm32f103-blinky/io-smoke.yaml` | deterministic integration smoke baseline | `green` | Coverage is narrow to smoke path |
| 5 | `firmware-rv32i-ci-fixture` | Generic RV32I | CI fixture | RV32I | `firmware-rv32i-ci-fixture` | `core/configs/systems/ci-fixture-riscv-uart1.yaml` | `core/examples/ci/riscv-uart-ok.yaml` | `gpio/uart/timer` CI baseline | `green` | Broader peripheral set |
| 6 | `stm32f401-blackpill` | STM32F4 | BlackPill F401CC | ARMv7E-M | `firmware-f401-demo` | `core/configs/systems/blackpill-f401cc.yaml` | `core/examples/blackpill-f401cc/uart-smoke.yaml` | `rcc/gpio/uart/systick` deterministic smoke path (reuses F401 firmware) | `green` | Deep peripheral breadth |
| 7 | `rp2040-pico` | RP2040 | Raspberry Pi Pico | Cortex-M0+ | `firmware-rp2040-pio-onboarding` | `core/configs/systems/pico.yaml` | `core/examples/rp2040-pio/asm-smoke.yaml` | `pio/gpio/uart` hardware fidelity baseline | `green` | Clock tree depth |
| 7.5 | `nrf52832-example` | nRF52832 | nRF52 DK (PCA10040) | ARMv7E-M | `firmware-nrf52832-demo` | `core/configs/systems/nrf52832-example.yaml` | `core/examples/nrf52832/uart-smoke.yaml` | `gpio/uart` deterministic smoke path | `green` | Timer/Radio/EasyDMA |
| 8 | `nrf52840-dk` | nRF52 | PCA10056 DK | ARMv7E-M | `firmware-nrf52840-demo` | `core/configs/systems/nrf52840-dk.yaml` | `core/examples/nrf52840-dk/uart-smoke.yaml` | `uart` baseline | `green` | Radio/PPI/EasyDMA |
| 9 | `stm32g474-nucleo` | STM32G4 | NUCLEO-G474RE | ARMv7E-M | `firmware-stm32g474-demo` | `core/configs/systems/nucleo-g474re.yaml` | `core/examples/nucleo-g474re/uart-smoke.yaml` | `rcc/gpio/uart/systick` baseline | `green` | Advanced timer/ADC depth |
| 10 | `stm32l476-nucleo` | STM32L4 | NUCLEO-L476RG | ARMv7E-M | `firmware-stm32l476-demo` | `core/configs/systems/nucleo-l476rg.yaml` | `core/examples/nucleo-l476rg/uart-smoke.yaml` | `rcc/gpio/uart/systick` baseline | `green` | Low-power clock tree |
| 11 | `stm32wb55-nucleo` | STM32WB | NUCLEO-WB55RG | ARMv7E-M | `firmware-stm32wb55-demo` | `core/configs/systems/nucleo-wb55rg.yaml` | `core/examples/nucleo-wb55rg/uart-smoke.yaml` | `gpio/uart` app-core baseline | `green` | Dual-core + radio |
| 12 | `atsamd21-xplained` | SAMD21 | Xplained Pro | ARMv6-M | `firmware-atsamd21-demo` | `core/configs/systems/atsamd21-xplained.yaml` | `core/examples/atsamd21-xplained/uart-smoke.yaml` | `uart` SERCOM baseline | `green` | GCLK/SERCOM depth |
| 13 | `atsame54-xplained` | SAME54 | Xplained Pro | ARMv7E-M | `firmware-atsame54-demo` | `core/configs/systems/atsame54-xplained.yaml` | `core/examples/atsame54-xplained/uart-smoke.yaml` | `uart` baseline | `green` | Clock tree depth |
| 14 | `efr32bg22-dk` | EFR32 | BRD4184 | ARMv8-M | `firmware-efr32bg22-demo` | `core/configs/systems/efr32bg22-dk.yaml` | `core/examples/efr32bg22-dk/uart-smoke.yaml` | `uart` USART baseline | `green` | Radio + performance |
| 15 | `gd32f103-board` | GD32F1 | Generic dev board | ARMv7-M | `firmware-gd32f103-demo` | `core/configs/systems/gd32f103-board.yaml` | `core/examples/gd32f103-board/uart-smoke.yaml` | `uart` F103 clone baseline | `green` | Vendor variant diffs |
| 16 | `ch32v003-board` | CH32V | CH32V003 EVB | RV32EC-ish | planned | planned | planned | Not started | `backlog` | ISA/peripheral variance |
| 17 | `fe310-hifive1` | SiFive FE310 | HiFive1 Rev B | RV32IMAC | `firmware-fe310-demo` | `core/configs/systems/fe310-hifive1.yaml` | `core/examples/fe310-hifive1/uart-smoke.yaml` | `uart` RISC-V baseline | `green` | HiFive1 hardware fidelity |
| 18 | `esp32c3-devkit` | ESP32-C3 | DevKit | RV32IMC | planned | planned | planned | Not started | `backlog` | Wi-Fi stack out of scope for smoke |
| 19 | `stm32u575-nucleo` | STM32U5 | NUCLEO-U575ZI-Q | ARMv8-M | `firmware-stm32u575-demo` | `core/configs/systems/nucleo-u575zi.yaml` | `core/examples/nucleo-u575zi/uart-smoke.yaml` | `uart` U5 modern baseline | `green` | Ultra-low power |
| 20 | `ra6m5-ek` | Renesas RA6 | EK-RA6M5 | ARMv8-M | `firmware-ra6m5-demo` | `core/configs/systems/ek-ra6m5.yaml` | `core/examples/ek-ra6m5/uart-smoke.yaml` | `uart` SCI baseline | `green` | TrustZone depth |

## Current Quarter Tracking Fields

Use this checklist per target:

- `chip descriptor`: exists and validated.
- `system manifest`: exists and validated.
- `smoke firmware`: deterministic UART or memory assertion.
- `unsupported instruction audit`: report generated.
- `known limitations`: documented and reviewed.
