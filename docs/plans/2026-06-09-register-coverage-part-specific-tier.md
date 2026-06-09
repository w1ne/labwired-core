# Register-coverage: the part-specific tier (DMA / AFIO / EXTI / RCC)

**Status:** **P0 + P1 (F103) DONE** (labwired-core #217) — DMA1/AFIO/EXTI/RCC
silicon-pinned on the bench F103, address sweep `diverge=0` (match=102). Key
unblocker: F103 is the *only* F1-family chip config, so AFIO (F1-only) and
`F1Rcc` are F103-exclusive — exact masks, no per-density gating. Only EXTI is
shared (F103/F401/L073), so its line count is a per-instance `config: {lines}`
field (F103=19, default 20). The multi-board probe-by-serial issue (below) was
the real blocker, not part-specificity. **P2 (F407) / P3 (F105/107) remain** —
blocked on those boards.
**Predecessor:** the per-family tier (TIM2 / USART2 / SPI1 / I2C1) is **done** —
18 silicon-validated mask/register fixes, F103 register-modeling coverage
210→221 (labwired-core PRs #214, #215, #216). Methodology:
`crates/hw-oracle/tests/stm32f1_mmio_diff.rs` address-only `SWEEP_CASES`.

## Why this tier is different (and was deliberately deferred)

The per-family tier worked because each peripheral's writable-bit mask is
**constant across the whole STM32 family** that shares the model (classic SPI is
identical on F1/F4/G4; legacy I2C on F1/F2/F4; the GP timer; the F1 USART). One
F103 bench reading pins the mask for every chip using that model.

DMA / AFIO / EXTI / RCC break that assumption — their masks are **part- and
density-specific**, while the models are shared:

| Peripheral | Why the mask varies by part | Shared model |
|---|---|---|
| **EXTI** | Implemented line count: F103 = 19 lines (`0x7FFFF`), F4-class = 23 | `ExtiRegisterLayout::Stm32F1` is explicitly shared F1 **+ F4** |
| **RCC** `AHBENR/APB2ENR/APB1ENR` | Writable bits = which peripherals the part physically has; F103-medium ≠ F105/F107 (connectivity) ≠ high-density | `F1Rcc` (all F1) |
| **AFIO** `MAPR/MAPR2` | Remap-bit set is line-specific; MAPR2 only exists on connectivity (F105/107) | `afio.rs` (all F1) |
| **DMA** `CCR` | Channel-config bits are universal, BUT the blunt `0xFFFFFFFF` probe returned `0x7AFF` (bits 8/10 dropped) — an `EN`-set-during-write artifact, needs a cleaner probe to characterise | `dma.rs` (F1/F4/L4) |

Applying an F103-derived mask to these shared models would **regress F407 /
F105 / F107 — chips with no bench to validate against**. That violates the
discipline ("every mask silicon-validated, never guessed") that makes the oracle
trustworthy. So this tier is gated on having those boards on hand.

## F103 silicon measurements already captured (do not re-measure)

Gathered on the bench F103 (Nucleo, ST-LINK/V2.1, 2026-06-09) via the sweep.
These are **silicon-true for F103-medium-density** — the only missing piece is
per-part gating so they don't leak onto other parts:

| Register | F103 silicon writable mask | Model today | Note |
|---|---|---|---|
| DMA `CCRx` | `0x7AFF` (re-probe: likely `0x7FFF` w/o EN race) | unmasked | reorder probe: CNDTR/CPAR/CMAR before CCR |
| DMA `CNDTRx` | `0xFFFF` | `0xFFFF` ✓ | reads 0 if CCR.EN set first (write-locked) |
| AFIO `EVCR` | `0x000000FF` | unmasked | |
| AFIO `MAPR` (SWJ held 0) | `0x0000FFFF` | `0x001FFFFF` | SWJ_CFG[26:24] probed separately, see denylist |
| AFIO `EXTICR1..4` | `0x00007FFF` | `0xFFFF` | |
| AFIO `MAPR2` | `0x0` (absent on F103-medium) | unmasked | present on F105/107 — part-specific |
| EXTI `IMR/EMR/RTSR/FTSR` | `0x0007FFFF` (19 lines) | `0x000FFFFF` (20) | F4 = 23 lines — part-specific |
| RCC `CIR` | `0x00001F00` (enable bits) | not stored (reads 0) | enables likely F1-universal |
| RCC `AHBENR` | `0x00000055` (DMA1/SRAM/FLITF/CRC) | unmasked | density-specific |
| RCC `APB2ENR` | `0x00005E7D` | unmasked | density-specific |
| RCC `APB1ENR` | `0x1AE64807` | unmasked | density-specific |

## Design: per-part mask gating

The models already select a **family** layout (`Stm32F1` etc.). This tier adds a
**part/density** discriminator within a family, since one family struct serves
several densities. Options, cheapest first:

1. **Per-part mask table keyed by chip id.** The chip yaml already names the part
   (`stm32f103`, `stm32f407`, …). Thread a small `PartMasks { exti_lines,
   rcc_ahbenr, rcc_apb1enr, rcc_apb2enr, afio_mapr, afio_mapr2, … }` into the
   peripheral model at construction (from the chip descriptor), defaulting to the
   widest/most-permissive when unknown. The model masks with the part value.
   Keeps one model struct per family; only a data table grows per part.
2. (rejected) A new layout enum per density — explodes the enum, duplicates code.

The mask table is **populated only from silicon-validated readings** — a part
with no bench keeps the permissive default (no regression, just not yet pinned).
F103 row is filled from the table above on day one.

## Self-destruct denylist (harness hardening — do FIRST, it's cheap + universal)

The address-only sweep writes `0xFFFFFFFF`; some bits are self-destructive. The
per-case `write` field already overrides the probe value. Formalise the list so
a cheap-model-enumerated address set can never brick the bench:

| Register.bits | Effect | Safe probe |
|---|---|---|
| AFIO `MAPR[26:24]` SWJ_CFG | `0b111` disables SWD/JTAG — **drops the debugger** | write `0xF8FFFFFF` (hold 0) |
| I2C `CR1.SWRST` (bit 15) | resets the peripheral | write `0x2CFB` (already done) |
| RCC `CFGR.SW` | switches SYSCLK — can hang | exclude (already) |
| RCC `*RSTR` | resets peripherals mid-test | exclude (already) |
| IWDG `KR` / FLASH `KEYR` | unlock-key sequences / watchdog start | exclude |

SWJ_CFG-disabled SWD is **recoverable** (power-cycle/reset re-enables it) — it
does not permanently brick.

## Multi-board bench protocol (cost ~hours of false "brick" panic this round)

With several ST-Links on USB, the test grabs the **first** probe, which changes
across replugs. Always pin the target by serial:

- **F103 Nucleo** → ST-LINK/V2.1 `0483:374b`, serial `066CFF534951775087071123`.
  Pass `LABWIRED_STLINK_SERIAL=<serial>` (the openocd helper applies `adapter
  serial`).
- A Seeed XIAO **nRF52840** (Cortex-M4) lives on a separate ST-LINK/V2 dongle
  `0483:3748` (garbage serial) — unrelated nRF onboarding.
- **Wrong-board tells:** CPUID `0x410FC241` (M4) / `DBGMCU@0xE0042000`=0 / erased
  flash vectors = that's the nRF, NOT a bricked F103. Healthy F103 = CPUID
  `0x411FC231` (M3), DBGMCU `0x20036410`. Enumerate serials:
  `cat /sys/bus/usb/devices/*/serial`.

## Phasing

- **P0 — harness hardening (no boards needed):** formalise the self-destruct
  denylist + the EN-locking probe order (DMA channel regs before CCR) + document
  the probe-by-serial protocol in the test. Land standalone; it makes every
  future sweep safe.
- **P1 — F103 part rows (board in hand):** add the `PartMasks` plumbing + fill the
  F103 row from the table above; gate the model masks on it; re-sweep → diverge=0;
  land. Bumps F103 coverage further (the BRR/CR2/GTPR-style "missing register"
  wins: RCC `CIR` storage, AFIO masks).
- **P2 — F407 (Cortex-M4): UNBLOCKED, on the bench (2026-06-09).** Scope refined
  after inspecting the chip: **F407 models only RCC / GPIO×4 / SysTick / UART /
  I2C — NO EXTI/DMA/AFIO**, so the "tier" registers don't apply to it. The real
  P2 value is therefore: (a) give F407 its **first** silicon oracle (it has
  none), (b) **cross-validate the shared UART (F1 USART layout) + I2C (`F1I2c`)
  masks on F4 silicon** — proving the per-family fixes are universal F1→F4, and
  (c) first-validate **F4Rcc** (known approximation: CFGR modelled at 0x04, real
  F4 has PLLCFGR@0x04 / CFGR@0x08 — the sweep will catch it) + `stm32f4_gpio`.
  Needs a **new `stm32f4_mmio_diff.rs`** (F4 memory map: RCC 0x40023800 with
  AHB1ENR@0x30/APB1ENR@0x40/APB2ENR@0x44, GPIO 0x40020000, USART2 0x40004400,
  I2C1 0x40005400, DBGMCU 0xE0042000).
  - **Bench access (done):** the F407 enumerates as a **garbage-serial clone
    ST-Link/V2 at USB location `1-1`** (the nRF52840 is the other clone dongle at
    `1-2`). Serial selection can't disambiguate them → added
    **`LABWIRED_STLINK_LOCATION`** (labwired-core #218). Confirmed F407 alive and
    readable on 1-1: `pc=0x08000040` (flash), `msp=0x20020000`, RCC_CR
    `0x00008283`, GPIOA_MODER `0xa8000000`. Connect with `stm32f4x.cfg`; its
    flash firmware runs but peripheral reads work after `reset halt`.
- **P3 — F105/F107 (connectivity):** AFIO MAPR2, the extra RCC enables. Lowest
  priority (least common parts).

## Exit

Each part's DMA/AFIO/EXTI/RCC writable masks silicon-pinned in its `PartMasks`
row, the shared models gated on it, and every chip's mmio-diff `diverge=0` on its
own bench — no mask guessed, none leaked across densities.
