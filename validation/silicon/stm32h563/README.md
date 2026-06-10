# STM32H563 silicon-validation corpus

Reset-state register captures from **real STM32H563 silicon** (NUCLEO-H563ZI,
on-board STLINK-V3, OpenOCD 0.12.0 via `stlink-dap` + `dapdirect_swd`,
Cortex-M33 on AP1). This is the ground-truth evidence behind the
silicon-verified reset values in `core/configs/chips/stm32h563.yaml`, the
`stm32h5` RCC model, and the `h563_conformance` gate
(`core/crates/core/tests/h563_conformance.rs`).

**Why this lives in the private repo:** per the moat boundary
(`docs/strategy/2026-06-06-moat-refinement-simulation-incumbents.md`), model
code → public `labwired-core`; the silicon-validation *evidence pipeline* →
private. The capture **script** is public
(`core/scripts/hw-capture-stm32h563.sh`); its **output** (these traces) is the
private corpus.

## Contents

- `hw-capture-<timestamp>/registers.txt` — labelled `mdw` dump: identity
  (DBGMCU_IDCODE, UID, FLASHSIZE), Cortex-M33 system registers, the full RCC
  reset surface at reset halt, and the reset values of every onboardable
  peripheral block (GPIO A–G, USART1–3, LPUART1, TIM1/2/3/6, SPI1–3, I2C1/2,
  GPDMA1/2, ADC1, RNG, CRC, WWDG, IWDG, RTC, LPTIM1, EXTI, PWR, FLASH,
  ICACHE) after enabling their bus clocks.
- `hw-capture-<timestamp>/stm32h563-dap.cfg` — the exact OpenOCD target
  config used (0.12.0 has no stm32h5x.cfg; hla cannot reach AP1).
- `hw-capture-<timestamp>/vcp-smoke.txt` — USART3 VCP output of the
  digital-twin smoke firmware (`core/examples/nucleo-h563zi/silicon-smoke`)
  running on the board; byte-identical to the simulator's output for the
  same ELF:
  `H563-SMOKE CR=0000002B MODERA=ABFFFFFF CALIB=001003E8` / `H563-SMOKE done`.
- `hw-capture-<timestamp>/probe-info.txt` — probe info.

## Reproduce

```bash
core/scripts/hw-capture-stm32h563.sh    # writes to core/fixtures/ (gitignored scratch)
# then curate the complete run into validation/silicon/stm32h563/ and commit here
```

## Provenance — capture 20260610-123455

- Board: NUCLEO-H563ZI
- Probe: on-board STLINK-V3 (V3J13M4, API v3), target voltage 3.289 V
- `DBGMCU_IDCODE` = `0x10016484` (DEV_ID 0x484 = STM32H563/573, REV_ID 0x1001)
- `CPUID` = `0x410FD214` (Cortex-M33 r0p4)
- `UID` = `00320047 31325118 33303933`
- `FLASHSIZE` = `0x0800` (2048 KB)
- Date: 2026-06-10

## Notable silicon facts captured

- RCC_CR reset `0x2B` (HSION|HSIRDY|HSIDIV=÷2|HSIDIVF); HSICFGR `0x004004F7`,
  CSICFGR `0x00200087` (TRIM defaults + per-part CAL); AHB1ENR reset
  `0xD0000100`, AHB2ENR `0xC0000000` (SRAM2/3 clocks on), RSR `0x0C000000`.
- GPIO reset values are per-port: A `0xABFFFFFF/0x0C000000/0x64000000`
  (SWD pins), B `0xFFFFFEBF/0xC0/0x100` (JTDO/NJTRST), C–G all-analog
  `0xFFFFFFFF`.
- SysTick CALIB = `0x001003E8`.
- WWDG counter reads live (`0x4D` < reset `0x7F`) once its APB clock is on,
  with WDGA clear — treat as volatile in any future conformance sweep.
- VTOR resets to `0x08000000` on this part (sim models 0 — known divergence,
  CPU-level).
- GPDMA1 at 0x40020000 (channel CxSR IDLEF=1 at +0x60) — the sim's generic
  7-channel DMA at that address is NOT silicon-conformant (documented in the
  chip yaml).
