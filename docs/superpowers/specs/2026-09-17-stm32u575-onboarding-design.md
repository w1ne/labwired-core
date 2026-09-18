# STM32U575ZI onboarding design

**Date:** 2026-09-17
**Status:** approved for implementation (user: approved full plan)
**Branch:** `feat/onboard-stm32u575` (worktree `/home/andrii/projects/labwired-u5-onboarding`)

## Purpose

Onboard STM32U575ZI (NUCLEO-U575ZI-Q) as a first-class LabWired target, and
validate it through the existing fidelity engine rather than a bespoke path:

1. **Datasheet-derived configs** — ST SVD + DS13736/RM0456, ingested with the
   repo's own SVD pipeline (`gen_debug_schemas.py` / `labwired asset ingest-svd`).
2. **Renode as reference where it exists** — Renode master has *no* STM32U5
   platform (verified against `platforms/cpus` on 2026-09-17; closest are
   `stm32wba52.repl` and `stm32l552.repl`). Those are used as behavioral
   references; the absence is documented instead of fabricating a golden run.
3. **Running HAL** — an unmodified-structure STM32CubeU5 HAL application
   (`Projects/NUCLEO-U575ZI-Q/Templates/TrustZoneDisabled` pattern) built with
   arm-none-eabi-gcc and executed on the simulator.
4. **Fidelity engine + smokes** — Arduino matrix (L0–L8), onboarding smoke
   workflow, unsupported-instruction audit, validation manifest, scoreboards.

Why U575: `docs/onboarding-candidates.md` Tier B names it ("mostly config +
fixture", ST's flagship low-power line, WBA52 already carries the U5-derived V2
RCC offsets) and row 19 of the monorepo matrix (`labwired/docs/specs/TOP20_COVERAGE_MATRIX.md`,
outside this core repo; its row-19 `green` status is aspirational/stale) already
reserves the artifact paths this spec uses.

## Target facts (sources pinned in artifacts)

| Fact | Value | Source |
|------|-------|--------|
| Core | Cortex-M33 r0p4 (TrustZone-capable, factory **disabled**) | UM2883 §"Factory default state is TrustZone disabled" |
| Flash | 2 MiB @ 0x0800_0000 | DS13736, Zephyr `stm32u575Xi.dtsi` |
| RAM | 768 KiB SRAM1+2+3 @ 0x2000_0000; SRAM4 16 KiB @ 0x2800_0000 | Zephyr dtsi; DS13736/RM0456; SVD memory map |
| RCC | 0x4602_0C00, V2 layout, PLL1CFGR@0x28 / PLL1DIVR@0x34 / PLL1FRACR@0x38, CFGR1@0x1C, enable map AHB1ENR@0x88…APB3ENR@0xA8 | SVD `STM32U575.svd` |
| PWR | 0x4602_0800, VOSR@0x0C (reset 0x8000), CR3 REGSEL/FSTEN | SVD |
| VCP console | **USART1** PA9/PA10 @115200 8N1, NVIC IRQ 61 | `stm32u5xx_nucleo.c` (`COM1`), Zephyr `nucleo_u575zi_q` docs, CubeU5 USBX README |
| Board LEDs / button | LD1=PC7, LD2=PB7, LD3=PG2, USER=PC13 | Zephyr board doc, UM2861 |
| Arduino `Serial` | **USART1** PA9/PA10 (`variant_NUCLEO_U575ZI_Q.h` overrides the generic variant: `SERIAL_UART_INSTANCE 1`) | STM32duino 3.0.0 variant header |
| Arduino `LED_BUILTIN` | PC7 (`LED_GREEN = LED_LD1`) | STM32duino 3.0.0 variant header |
| DBGMCU | 0xE004_4000, IDCODE reset 0x3001_6482 (DEV_ID 0x482) | SVD |
| Max clock | 160 MHz (PLL1 from MSI 4 MHz) | DS13736, CubeU5 template |

## Deliverables

### Configs

- `tests/fixtures/real_world/stm32u575.svd` — vendored ST SVD (cmsis-svd-data).
- `configs/peripherals/stm32u575/*.yaml` — debug schemas generated from the SVD.
- `configs/chips/stm32u575.yaml` — core/flash/RAM (`memory_regions` for SRAM4,
  mirroring `stm32h735.yaml`), RCC/PWR/FLASH/GPIO/USART/LPUART/UART4/I2C/SPI/
  TIM/GPDMA/ADC/RTC/IWDG/ICACHE/CRC/RNG/DBGMCU/NVIC with SVD bases + IRQs.
  Every divergence from silicon documented inline, like `stm32h563.yaml`.
- `configs/systems/nucleo-u575zi.yaml` — NUCLEO-U575ZI-Q: USART1 VCP console,
  LEDs PC7/PB7/PG2, button PC13, SRAM4 window. Same console UART serves the
  Cube HAL app and the Arduino core (variant header maps `Serial` to USART1).

### Firmware

- `crates/firmware-stm32u575-demo` — minimal Rust smoke (`OK\n` over USART1),
  built `thumbv8m.main-none-eabi`, named per TOP20 row 19, wired into
  `.github/workflows/core-onboarding-smoke.yml`.
- `examples/nucleo-u575zi/board_firmware/` — STM32CubeU5 HAL C app:
  `HAL_Init` → `HAL_RCC_OscConfig`/`HAL_RCC_ClockConfig` (MSI→PLL1→160 MHz,
  VOS1, SMPS) → `HAL_GPIO_Init` (LD1 PC7) / `HAL_UART_Init` (USART1 PA9/PA10,
  115200) → `HAL_UART_Transmit` banner + `HAL_Delay` blink loop. Uses the stock
  HAL drivers directly rather than the Nucleo BSP (fewer vendored files, the
  same HAL register flows; BSP is an optional follow-up). Makefile mirrors
  `examples/nucleo-h563zi/board_firmware/Makefile` with
  `STM32CUBE_U5_DIR ?= $(abspath ../../../../STM32CubeU5)` (repo-root sibling,
  same convention as H563's `STM32CubeH5` — no new fetch script). Checkout
  `git clone --depth 1 https://github.com/STMicroelectronics/STM32CubeU5`
  at the repo root before validation step 3; version/revision is recorded in
  `VALIDATION.md` when the HAL run is captured. TrustZone-disabled flash alias
  0x0800_0000 (factory state).

### Example + docs

- `examples/nucleo-u575zi/{README.md,system.yaml,uart-smoke.yaml,VALIDATION.md,REQUIRED_DOCS.md,EXTERNAL_COMPONENTS.md}`
- `docs/boards/stm32u575.md` from `docs/boards/_TEMPLATE.md`, honest ✅/⚠️/❌.

### Fidelity engine hooks

- `validation/arduino-matrix/systems/stm32u575.yaml` — INA219 @0x40 on i2c1,
  MAX31855 on spi1 (same kit as every STM32 sibling).
- `validation/arduino-matrix/boards.yaml` entry:
  `pio: { platform: ststm32, board: nucleo_u575zi_q, framework: arduino }`,
  `max_steps` + `budget_reason` (the file header makes these mandatory; start
  from the H563/L476 budget, `20000000`, reason "L0 light; L3 Wire.begin
  i2c_computeTiming needs multi-M steps", tighten with a measured number after
  the first run), `led_watch: "gpioc:7"` (LED_BUILTIN = PC7 green), documented
  skips (expected: L5/L8 until ADC/FDCAN are modeled; each skip carries a reason).
- `validation/manifest.yaml` entry `id: stm32u575` with `doc: docs/boards/stm32u575.md`,
  `chip: configs/chips/stm32u575.yaml`, `tier: sim-validated` (no bench part),
  `offline_tests`, `note`, and a `models:` list that covers **every source path
  the chip YAML wires** (the drift-watch generator fails on missing paths, e.g.
  `crates/core/src/peripherals/rcc.rs`, `pwr.rs`, `flash.rs`, `uart.rs`,
  `gpio.rs`, `i2c.rs`, `spi.rs`, `timer.rs`, `adc.rs`, `rtc.rs`, `iwdg.rs`,
  `gpdma.rs`, `crc.rs`, `rng.rs`, `dbgmcu.rs`, `configs/peripherals/stm32u575`).
  Regenerate `docs/boards/VALIDATION_STATUS.md`.
- `.github/workflows/core-onboarding-smoke.yml` matrix entry with the uart-smoke
  script (crate `firmware-stm32u575-demo`).
- Regenerated `docs/coverage/arduino-scoreboard.md` (and tier1/firmware-exercise
  docs only if their generators change output).

## Simulator model work anticipated

From reading the shared models, in likely order of appearance:

1. **RCC V2/U5**: `V2Rcc` today treats offset 0x28 as a request/ack hack that
   is wrong once 0x28 is `PLL1CFGR` (true on both WBA and U5). Add a U5/WBA-safe
   PLL1 register block (CFGR/DIVR/FRACR storage + `PLL1ON→PLL1RDY` in CR via the
   existing `classic_cr_ready` path), CFGR2@0x20 HPRE/PPRE storage, and the U5
   enable map if `V2EnrMap` differs from WBA. **WBA52 and G474 regressions are
   mandatory** for every RCC change.
2. **PWR U5**: VOSR@0x0C write → VOSRDY (extend `PwrWba`, new `profile: u5` if
   the offset/bit layout differs), CR3 supply selection round-trip.
3. **FLASH**: ACR latency write/read-back (HAL `HAL_RCC_ClockConfig` requires
   it); reuse the L4/H5-style profile; keep program/erase out of scope.
4. **UART**: `UartRegisterLayout::Stm32V2` already places ISR 0x1C / RDR 0x24 /
   TDR 0x28 / BRR 0x0C like U5 — verify PRESC@0x2C storage (currently
   unhandled), ICR W1C, and TX path with the HAL's `HAL_UART_Transmit` loop.
5. **I2C / SPI** (Arduino L3/L4): reuse the sibling profiles WBA52 already
   wires — `i2c: profile stm32l4` (I2C1 0x4000_5400, IRQ 43 on WBA; U5 IRQ 43),
   `spi: profile stm32h5` (SPI1 0x4001_3000) — and confirm the U5 pad map
   (PA5/PA6/PA7 / PA4 NSS) for the matrix kit before claiming L4.
6. **ICACHE / GPIO / DBGMCU / SysTick / NVIC**: reuse WBA52 models; DBGMCU
   IDCODE 0x3001_6482.

No new CPU work is expected: M33 + TrustZone-agnostic execution is already
proven by WBA52.

## Validation (definition of done)

Run from the worktree root unless noted:

```bash
# 1. targeted model tests + STM32 regressions
cargo test -p labwired-core <new tests> -- --nocapture
cargo test -p labwired-core h563 -- --nocapture
cargo test -p labwired-core wba52 -- --nocapture

# 2. Rust smoke + CLI
cargo run -q -p labwired-cli -- test --script examples/nucleo-u575zi/uart-smoke.yaml

# 3. CubeU5 HAL firmware (real vendor HAL)
make -C examples/nucleo-u575zi/board_firmware
cargo run -q -p labwired-cli -- \
  --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 20000000
# evidence: banner over USART1; two runs byte-identical

# 4. unsupported-instruction audit
./scripts/unsupported_instruction_audit.sh \
  --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system configs/systems/nucleo-u575zi.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo-u575zi

# 5. Arduino matrix (fidelity engine)
cargo build -p labwired-cli --release
python3 validation/arduino-matrix/run_matrix.py --boards stm32u575

# 6. docs/scoreboards freshness
python3 scripts/generate_validation_status.py --check
```

Done means: 1–6 pass, skips are documented with reasons, and
`examples/nucleo-u575zi/VALIDATION.md` records exact commands + outputs.

## Non-goals / honesty rules

- **No silicon claims.** No bench part is attached: manifest tier is
  `sim-validated`, `docs/boards/stm32u575.md` says so.
- TrustZone enforcement, GTZC/SAU, OCTOSPI/HSPI, USB, ethernet, FDCAN data
  path, full ADC/DAC/DMA feature sets are out of scope for this onboarding;
  the chip YAML declares what exists and what is deliberately absent.
- No tier-1 rubric fixture in this change (Arduino L0–L8 + HAL cover the boot
  path); a tier-1 fixture is a follow-up with its own budget.
- Renode is a reference, not an oracle here: no U5 REPL exists upstream, so no
  Renode-vs-LabWired differential is claimed.

## Risks

1. CubeU5 HAL bring-up may surface shared-model bugs (expected; that is the
   validation value). Each fix needs WBA52/H563/L476 regression coverage.
2. STM32duino's U575 support may hit core-level issues unrelated to our models
   (Serial on UART4, `CUSTOM_PERIPHERAL_PINS`); matrix status will classify
   those honestly rather than papering over.
3. `V2Rcc` 0x28 PLL1CFGR fix is cross-family: WBA52 behavior must stay
   byte-identical or its tests must be updated with justification.
