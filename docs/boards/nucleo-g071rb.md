# NUCLEO-G071RB (ST STM32G071RB) — UART/LED smoke

The customer-board onboarding lane adds a narrow, board-connected profile at
[`configs/systems/nucleo-g071rb-smoke.yaml`](../../configs/systems/nucleo-g071rb-smoke.yaml).
It exposes only RCC, GPIOA, USART2 and the LD4 LED wiring, and is exercised by
the committed USART2/PA5 smoke in
[`examples/nucleo-g071rb/evidence/`](../../examples/nucleo-g071rb/evidence/).
The live vendor status receipt is included there. This lane does not claim
support for the board button, ST-LINK itself, USB, or peripherals outside that
profile.

An ST **STM32G071RB** Nucleo-64 board — single-core **Cortex-M0+ (ARMv6-M)**,
128 KB flash, 36 KB SRAM, up to 64 MHz. This page covers the modelled slice:
core + SysTick, the **G0 RCC** layout, GPIO on the `0x50000000` IOPORT bus,
and USART2 (the ST-LINK VCP console). Built for `thumbv6m-none-eabi`.

**Tier: L3 production-ready (SIM-DERIVED).** The six tier-1 classes —
clock/RCC, GPIO, UART, Timer, DMA and interrupt delivery — all pass for the
documented Tier-1 scenarios, on top of stable CI and the reviewed instruction
audit. No STM32G0 silicon has ever been probed over SWD for this port; every
register number traces to RM0444 / DS12232 / ST's CMSIS `stm32g071xx.h`.
Read [`examples/nucleo-g071rb/KNOWN_LIMITATIONS.md`](../../examples/nucleo-g071rb/KNOWN_LIMITATIONS.md)
before trusting any peripheral beyond the proven scenarios.

The **Tier-1 fixture** (`tests/fixtures/tier1/stm32g071.elf`) raw-register
self-tests every class over the USART2 console: clock, gpio, timer, pwm, dma,
irq, i2c, spi, adc, wdt and rtc all pass, plus the implicit uart proof
(eleven explicit classes + implicit uart = all twelve matrix cells, `TIER1
done`).

!!! tip "Live status"
    Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match
    - [Bus visibility](../coverage/bus-visibility.md) — which buses produce decodable edges
    - [Target support rubric](../target_support_rubric.md) — modeled / stub / silicon-verified

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/stm32g071.yaml`](../../configs/chips/stm32g071.yaml) |
| Example system | [`configs/systems/nucleo-g071rb.yaml`](../../configs/systems/nucleo-g071rb.yaml) |
| Reference firmware | [`crates/firmware-stm32g0-demo/`](../../crates/firmware-stm32g0-demo/) |
| Committed ELF | `tests/fixtures/nucleo-g071rb-smoke.elf` |
| Tier-1 fixture | `tests/fixtures/tier1/stm32g071.elf` — 11 classes `pass` + implicit `uart` (all 12 matrix cells) |
| Example / evidence | [`examples/nucleo-g071rb/`](../../examples/nucleo-g071rb/) |
| Known limitations | [`examples/nucleo-g071rb/KNOWN_LIMITATIONS.md`](../../examples/nucleo-g071rb/KNOWN_LIMITATIONS.md) |
| Validation | `firmware_survival::test_nucleo_g071rb_smoke_survival` · `stm32g071_from_config_builds` · Tier-1 matrix row |
| Tier | **L3 production-ready** — L1 smoke + stable CI + instruction audit + all six tier-1 peripherals pass; **no silicon diff** |
| Playground board id | none yet (not in the bundled catalog) |

---

## Flash / firmware artifact

| Use | Artifact | Notes |
|-----|----------|-------|
| CLI / survival fixture | `tests/fixtures/nucleo-g071rb-smoke.elf` | Built `--release --target thumbv6m-none-eabi` from `crates/firmware-stm32g0-demo` |
| Rebuild | `cargo build -p firmware-stm32g0-demo --release --target thumbv6m-none-eabi` | Build.rs passes its own `-Tlink.x`; no `.cargo/config.toml` needed |
| Real board | `objcopy -O binary` then flash at `0x08000000` | The same binary is intended to run on silicon |

---

## Pins (NUCLEO-G071RB, UM2505 / Zephyr `nucleo_g071rb` DTS)

| Board label | MCU pin | Function | Notes |
|-------------|---------|----------|-------|
| LD4 (green) | PA5 | GPIO output | active high, toggled by the smoke |
| B1 (user) | PC13 | GPIO input | active low (pull-up to VDD) |
| USART2_TX | PA2 | AF1 | ST-LINK V3 Virtual COM Port |
| USART2_RX | PA3 | AF1 | ST-LINK V3 Virtual COM Port |
| SWDIO / SWCLK | PA13 / PA14 | SWD | Cortex-M0+ has **no JTAG TAP** |
| Arduino I2C | PB8 / PB9 | AF | declared in the chip yaml, unproven |
| Arduino SPI | PB0/PA5/PA6/PA7 | AF | PA5 shared with LD4, unproven |

---

## Support matrix

| Mark | Meaning |
|------|---------|
| ✅ | Modeled well enough for the firmware this repo ships |
| ⚠️ | Present but partial / family-reused / not G0-tuned |
| ❌ | Not simulated — use the bench |

### Core & boot

| Block | Status | Notes |
|-------|--------|-------|
| Cortex-M0+ CPU | ⚠️ | Decoder executes the full Thumb-2 set; the ARMv6-M subset is enforced only by the `thumbv6m` toolchain |
| Flash / SRAM windows | ✅ | 128 KB @ `0x08000000` / 36 KB @ `0x20000000` (DS12232) |
| SysTick | ✅ | |
| NVIC | ✅ | |
| DBGMCU | ⚠️ | APB @ `0x40015800`; DEV_ID `0x460` is ST's published value, **not read off a die** |

### Clock & reset

| Block | Status | Notes |
|-------|--------|-------|
| RCC `stm32g0` layout | ✅ | CR/ICSCR/CFGR/PLLCFGR @0x00–0x0C, 3-bit SW/SWS, IOPENR@0x34 / AHBENR@0x38 / APBENR1@0x3C / APBENR2@0x40, BDCR@0x5C / CSR@0x60 — verified against RM0444 §5.4 + `stm32g071xx.h`, pinned by unit tests |
| HSI16 ready / SYSCLK switch | ✅ | HSION bit8 → HSIRDY bit10; SW→SWS completes only when the source is ready |
| LSI / LSE ack | ✅ | CSR bit0→bit1, BDCR bit0→bit1 |
| PLL frequency | ❌ | PLLON→PLLRDY is modelled; the programmed frequency is not |

### Console, GPIO, timers

| Block | Status | Notes |
|-------|--------|-------|
| USART2 (VCP) | ✅ | `stm32v2` USART layout, clocked via APBENR1 bit17; byte path proven by the smoke |
| USART1/3/4, LPUART1 | ⚠️ | Same model, declared; not exercised end-to-end |
| GPIO A–D, F | ✅ | `stm32v2` layout on the IOPORT bus; LD4/PA5 + B1/PC13 declared in `board_io` |
| TIM1/2/3/6/7/14/15/16/17 | ⚠️ | Family timer model; G0 widths declared (TIM2 32-bit, rest 16-bit) but not timed against silicon. Tier-1 proves TIM2 32-bit ARR/UIF/CEN counting and TIM1 advanced compare-flag latching + CC1IF |
| LPTIM1/2 | ⚠️ | Declared; family model, unproven |

### Buses & analog

| Block | Status | Notes |
|-------|--------|-------|
| I2C1 / I2C2 | ⚠️ | `stm32l4` controller model reused — not G0-diffed. Tier-1 proves I2C1 PE/BUSY/STOP and an absent-slave NACK transaction |
| SPI1 / SPI2 | ⚠️ | Classic (`stm32`) FIFO-less model reused — not G0-diffed. Tier-1 proves SPI1 TXE→BSY→completion + RXNE |
| ADC1 | ⚠️ | `stm32l4` ADC model: conversion by value from a fixed internal source (3.0 V / 3.3 V → 3723 at 12-bit, scaling with `CFGR.RES`) — proven by Tier-1, **not** G0-diffed and no external analog input |
| DAC1 | ❌ | Register window only; analog output not modelled for this part |
| DMA1 | ⚠️ | Family DMA model, declared; DMAMUX not declared. Tier-1 proves a mem-to-mem byte copy + TCIF1 |
| RTC / IWDG / WWDG / CRC | ⚠️ | Family models, declared; CRC is 32-bit IDR. Tier-1 proves RTC DR reset/WPR unlock/TR write and IWDG PR/RLR write-protection |
| EXTI / SYSCFG | ⚠️ | Single-bank EXTI (`stm32f1` profile); SYSCFG model present |
| UCPD / CEC / VREFBUF / COMP | ❌ | Not declared / not modelled |

### What the smoke actually proves

1. The G0 RCC map: firmware writes IOPENR (`0x34`) and APBENR1 (`0x3C`);
   a wrong (L0/L4) offset leaves the UART silent.
2. `HSIRDY` at CR bit 10 — the G0 ready-bit layout.
3. USART2 TDR writes leave the part as UART bytes (`OK`).
4. LD4 (PA5) toggles through GPIOA BSRR under the G0 IOPENR gate.

### What it does **not** prove

- Anything about real silicon: there is no NUCLEO-G071RB capture in this
  repo, and no executing-fidelity differential (no walk-vs-scheduler oracle).
- Clock frequencies, baud timing, DMA/DMAMUX, analog, or the non-console
  buses.
- Interrupt delivery for any IRQ (the smoke is polled, not interrupt-driven).

### Tier-1 peripheral self-tests

`examples/tier1-fixture/stm32g071/` is a standalone `no_std` fixture built for
`thumbv6m-none-eabi` that pokes the peripherals raw-register (RM0444 offsets)
and prints the TIER1 protocol over USART2. The committed blob is
`tests/fixtures/tier1/stm32g071.elf`.

```bash
labwired run --chip configs/chips/stm32g071.yaml \
  --firmware tests/fixtures/tier1/stm32g071.elf --max-steps 8000000
```

| Class | Matrix cell | What the fixture proves |
|-------|-------------|-------------------------|
| clock | pass | CR HSION/HSIRDY bits 8/10, HSEON→HSERDY 16/17, 3-bit CFGR.SW/SWS (reserved encodings hold SWS), IOPENR/AHBENR/APBENR1/APBENR2 set/clear round-trips, GPIOC dead while IOPENR-gated |
| gpio | pass | GPIOA dead while gated; MODER/OTYPER round-trips, BSRR set → ODR + IDR, BRR and BSRR-high reset |
| timer | pass | TIM2 (32-bit) dead while gated; 32-bit ARR round-trip, EGR.UG→UIF, SR rc_w0, CEN→CNT advances |
| pwm | pass | TIM1 advanced (`tim1_pwm`, APBENR2 bit11) dead while gated; UG latches CC2..4IF/CC5..6IF but not CC1IF; running counter raises CC1IF after CCR1 with BDTR.MOE set. The `_pwm` id suffix declares the class to `declared_classes_from_yaml`, so the cell renders (SVD block is still TIM1 via the alias stem) |
| dma | pass | DMA1 dead while gated; CH1 mem-to-mem copy (MINC+PINC) with matching data + TCIF1 |
| irq | pass | NVIC software-pended IRQ 30 actually enters the vector handler |
| i2c | pass | I2C1 dead while gated; PE round-trip, START→BUSY, STOP clears, absent-slave NACK + AUTOEND release |
| spi | pass | SPI1 dead while gated; TXE at idle, DR write → BSY, completion re-asserts TXE and raises RXNE |
| adc | pass | ADC1 dead while gated; DEEPPWD/ADVREGEN/ADEN→ADRDY sequencing and a real conversion by value (3723 @12-bit, 930 @10-bit) |
| wdt | pass | IWDG reset PR/RLR, write protection without the 0x5555 key, unlock/latch, re-protect |
| rtc | pass | RTC DR dead while APB-gated, 0x2101 reset, WPR 0xCA/0x53 unlock, TR round-trip |
| uart | pass | Implicit: the TIER1 lines arriving over USART2 are the proof (console bring-up checks the APBENR1.USART2EN gate itself) |

Matrix visibility: after the `tim1_pwm` rename a live `tier1-matrix` run
records all twelve G071 cells `pass` (`pwm` was `na`), with no other chip's
recorded `pass` cells regressing.

Honest limits of the tier-1 row: it is simulator-side only (no bench capture);
only the first instance per class is swept (GPIOA, USART2, I2C1, SPI1, ADC1,
DMA1 ch1, TIM1/TIM2, IWDG, RTC); the PWM proof is flag-level, not a waveform
sweep (complementary outputs/dead-time/break untested); I2C/SPI are
controller-only (no external slave is wired, the I2C check deliberately
exercises an absent-slave NACK); the ADC converts the model's fixed internal
source, not a pin; timing is functional, not cycle-accurate. The full,
itemised list is in
[`examples/nucleo-g071rb/KNOWN_LIMITATIONS.md`](../../examples/nucleo-g071rb/KNOWN_LIMITATIONS.md).

### Instruction audit (L2)

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/tier1/stm32g071.elf \
  --system configs/systems/nucleo-g071rb.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo-g071rb
```

Observed: `199999` instructions executed, `unknown_thumb16: 0`,
`unhandled_thumb32: 0`, `unknown_riscv: 0`, `unsupported_total: 0`, exit 0 —
`100.0000%` instruction support on the audited window. The report is
`out/unsupported-audit/nucleo-g071rb/report.md`; the smoke path was audited
the same way (200000 instructions, 0 unsupported). Full transcript in
[`examples/nucleo-g071rb/VALIDATION.md`](../../examples/nucleo-g071rb/VALIDATION.md).

### CI coverage

| Lane | Workflow | What it gates |
|------|----------|---------------|
| Smoke matrix cell `nucleo-g071rb` | `.github/workflows/core-coverage-matrix-smoke.yml` | Builds `firmware-stm32g0-demo` (thumbv6m) and runs `examples/nucleo-g071rb/uart-smoke.yaml` on the example system |
| Tier-1 matrix + ratchet | `.github/workflows/core-ci.yml` (`tier1_matrix`, `tier1_matrix_ratchet`) | Runs every committed Tier-1 fixture live and fails any `pass → non-pass` drop against the merge-base baseline |
| Tier-1 fixture drift | `.github/workflows/core-nightly.yml` (`tier1-fixture-drift`) | Rebuilds all Tier-1 blobs (incl. `stm32g071.elf` via `scripts/tier1/build_stm32.sh`) and fails if a sha256 drifts from `tests/fixtures/tier1/MANIFEST.json` |
| Scoreboard staleness | `.github/workflows/core-board-ci.yml` (`coverage-staleness`) | Regenerates `docs/coverage/tier1-scoreboard.md` from the matrix and fails if stale |

---

## How to run

```bash
# Build the smoke firmware (Cortex-M0+)
cargo build -p firmware-stm32g0-demo --release --target thumbv6m-none-eabi

# Direct run — expect "OK" on the UART
cargo run -q -p labwired-cli -- \
  --firmware tests/fixtures/nucleo-g071rb-smoke.elf \
  --system configs/systems/nucleo-g071rb.yaml \
  --max-steps 200000

# Asserted smoke
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-g071rb/uart-smoke.yaml \
  --output-dir out/nucleo-g071rb/uart-smoke --no-uart-stdout
```

**Playground:** not registered in the bundled catalog — this board is CLI /
library only today. **MCP:** the board is reachable through the same
`Session`/bus APIs as any other chip; no board-specific MCP wiring exists.

---

## Related

- [NUCLEO-L073RZ](nucleo-l073rz.md) — the closest cheap STM32 (M0+, IOPORT
  bus, modern USART); the L0 RCC map differs from G0 (CRRCR@0x08, IOPENR@0x2C)
- [NUCLEO-G474RE / STM32G474RE](stm32g474re.md) — the G4 sibling, different
  RCC map and GPIO bus (`0x48000000`)
- [`examples/nucleo-g071rb/VALIDATION.md`](../../examples/nucleo-g071rb/VALIDATION.md) — exact commands + evidence
- [`examples/nucleo-g071rb/KNOWN_LIMITATIONS.md`](../../examples/nucleo-g071rb/KNOWN_LIMITATIONS.md) — what is not modelled / partially modelled / proven at L3
