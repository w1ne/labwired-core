# STM32F401-Nucleo Onboarding Experience Report (2026-02-22)

## Scope

Onboarding experience report for `stm32f401-nucleo` only.
Focus is limited to board onboarding phases (`P0..P5`) and board-specific blockers.

## Objective

Move `stm32f401-nucleo` from `backlog` to deterministic smoke-ready onboarding state.

Status: completed for minimal deterministic bring-up scope (`rcc + gpio + uart + systick`).

## Current Phase Status (P0..P5)

### P0 - Source grounding

Completed:
1. ST project path identified: `STM32F401RE-Nucleo`.
2. UART mapping confirmed from ST UART example:
   - peripheral: `USART2`
   - pins: `PA2` (TX), `PA3` (RX)
3. CMSIS constants captured:
   - `FLASH_BASE = 0x08000000`, `SRAM1_BASE = 0x20000000`
   - `USART2_BASE = 0x40004400`, `RCC_BASE = 0x40023800`, `GPIOA_BASE = 0x40020000`, `GPIOC_BASE = 0x40020800`
   - `USART2_IRQn = 38`
4. BSP board mappings captured:
   - LED2: `PA5`
   - USER button: `PC13`

### P1 - Engine fit

Completed:
1. Minimal viable bring-up target remains valid: `rcc + gpio + uart + systick`.
2. Chip/system config alignment verified for VCP path on `USART2`.

### P2 - Implementation

Completed:
1. Updated chip descriptor:
   - `core/configs/chips/stm32f401.yaml`
2. Added board system manifest:
   - `core/configs/systems/nucleo-f401re.yaml`
3. Added smoke firmware crate:
   - `core/crates/firmware-f401-demo/`
4. Added CI matrix integration entry for board smoke/audit:
   - `core/.github/workflows/core-coverage-matrix-smoke.yml`

### P3 - Example docs package

Completed:
1. `core/examples/nucleo-f401re/system.yaml`
2. `core/examples/nucleo-f401re/README.md`
3. `core/examples/nucleo-f401re/REQUIRED_DOCS.md`
4. `core/examples/nucleo-f401re/EXTERNAL_COMPONENTS.md`
5. `core/examples/nucleo-f401re/VALIDATION.md`
6. `core/examples/nucleo-f401re/io-smoke.yaml`
7. `core/examples/nucleo-f401re/uart-smoke.yaml`

### P4 - Validation

Completed:
1. Firmware built:
   - `cargo build -p firmware-f401-demo --release --target thumbv7em-none-eabi`
2. Deterministic smoke run:
   - `cargo run -q -p labwired-cli -- test --script examples/nucleo-f401re/uart-smoke.yaml --output-dir out/nucleo-f401re/uart-smoke --no-uart-stdout`
3. Direct run evidence:
   - `Initial PC: 0x800000a, SP: 0x20018000`
   - UART output: `OK`
4. Unsupported-instruction audit:
   - output dir: `core/out/unsupported-audit/nucleo-f401re`
   - `unsupported_total: 0`
   - `instruction_support_percent: 100.0000%`
5. Strict onboarding test re-run after firmware build:
   - `cargo test -p labwired-core test_strict_board_onboarding -- --nocapture`
   - includes: `[PASS] stm32f401 is strictly onboarded.`

### P5 - Report

Completed in this report revision.

## Onboarding-Specific Friction

1. Vendor source extraction friction:
   - STM32CubeF4 uses nested submodules; source acquisition for CMSIS/BSP requires explicit submodule-aware workflow.
2. Board mapping ambiguity at start:
   - multiple STM32F4 Nucleo variants exist; strict board ID (`STM32F401RE-Nucleo`) must be fixed first to avoid wrong BSP assumptions.
3. Existing chip config reuse risk:
   - pre-existing `stm32f401.yaml` can accelerate onboarding, but still needs board-path validation for UART and reset flow.

## Board Facts Captured

From ST UART example (`STM32F401RE-Nucleo/Examples/UART/UART_Printf`):
1. UART instance: `USART2`
2. TX pin: `PA2`
3. RX pin: `PA3`
4. Example system clock path targets 84 MHz from HSI + PLL

## Source References Used

1. MCU CMSIS header (memory map + IRQ):
   - https://github.com/STMicroelectronics/cmsis_device_f4/blob/master/Include/stm32f401xe.h
2. Nucleo BSP header (LED/button mapping):
   - https://github.com/STMicroelectronics/stm32f4xx-nucleo-bsp/blob/main/stm32f4xx_nucleo.h
3. Board UART example mapping (`USART2`, `PA2/PA3`):
   - https://github.com/STMicroelectronics/STM32CubeF4/blob/master/Projects/STM32F401RE-Nucleo/Examples/UART/UART_Printf/Inc/main.h
4. Board UART example clock intent:
   - https://github.com/STMicroelectronics/STM32CubeF4/blob/master/Projects/STM32F401RE-Nucleo/Examples/UART/UART_Printf/Src/main.c

## Remaining Work Beyond Minimal Bring-Up

1. Expand peripheral depth toward Tier-1 expectations (`timer`, `dma`) for F401 scenarios.
2. Add broader firmware compatibility examples (HAL/RTOS variants) on this board.
3. Promote scoreboard state from `scheduled` to `green` after repeated matrix evidence.
