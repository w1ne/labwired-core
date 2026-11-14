# Onboarding candidates

Which chips are worth adding next, and what each actually costs given the models
already in the tree. Ordered by cost, not by preference — a Tier A part is often
a weekend's work reusing shipped models, while a Tier C part means a new vendor
family from scratch.

Companion to [`board_onboarding_playbook.md`](board_onboarding_playbook.md),
which describes *how* to onboard. This file is *what* to onboard.

*Last updated: 2026-09-19 — nRF52833 (micro:bit v2), STM32G071 (NUCLEO-G071RB)
and ESP32-C6 (ESP32-C6-DevKitC-1) landed at L1 smoke. `configs/chips/` now
carries 41 descriptors.*

## What the engine already supports

Onboarding cost is dominated by how much of this a candidate can reuse.

| Axis | Supported today |
|------|-----------------|
| Cores | Cortex-M0+, M3, M4, M7, M33; RISC-V (RV32IMC executed; RV32IMAC decodes); Xtensa LX6 / LX7; AVR 8-bit (`avr8`) |
| STM32 RCC families | F1, F4, V2 (H5/WBA), H5, H7, L4, L0, G4, G0, WB, U5 |
| Vendors | ST, Nordic, Espressif, Raspberry Pi, NXP (Kinetis + i.MX RT), Microchip/Atmel (AVR, SAMD21/51), Renesas (RA4M1), Silicon Labs (EFR32MG26) |
| **Absent vendors** | **WCH, TI, Infineon** |

A candidate whose core is already supported and whose RCC/clock tree matches an
existing family is cheap. A candidate that needs a new core *or* a new vendor
clock tree is not.

## Tier A — near-free (reuses a shipped family)

These need a chip config, a tier-1 fixture and a validation entry. No new
peripheral models, or close to none.

| Candidate | Board it unlocks | Reuses | Notes |
|-----------|------------------|--------|-------|
| **nRF54L05 / L10** | — | nRF54L15 / LM20A onboarded; siblings differ in memory size | The cheapest remaining add on this list. SVDs already fetched for the family. |
| **STM32F446** | NUCLEO-F446RE | F4 family (F401/F407/F411 shipped) | Popular F4 SKU; same RCC layout. |
| STM32H723 / H725 / H730 | NUCLEO-H723ZG | H7 family (H735 shipped, RM0468) | Same RCC layout; H735 work carries over directly. |
| STM32L432 / L452 | NUCLEO-L432KC, Feather STM32L4 | L4 family (L476 silicon-verified) | |
| STM32G431 | NUCLEO-G431RB | G4 family (G474 shipped) | Cheapest STM32 in the modern line. |
| STM32L071 / L031 | NUCLEO-L031K6 | L0 family (L073 silicon-verified) | |

## Tier B — moderate (new peripherals, existing core + arch)

New register sets to model, but no new CPU and no new vendor conventions.

| Candidate | Board it unlocks | Reuses | New work |
|-----------|------------------|--------|----------|
| **ESP32-H2** | ESP32-H2-DevKitM | RISC-V + the C6 peripheral map that just landed | Thread/Zigbee focus; near-sibling of C6. |
| ESP32-S2 | ESP32-S2 Saola | Xtensa LX7 (S3 shipped) | Single-core S3 sibling. |
| STM32C0 | NUCLEO-C031C6 | M0+; the G0 RCC is the closest shipped layout | ST's cheapest new line; small peripheral set. |
| STM32L552 / L562 | NUCLEO-L552ZE-Q | M33 (U5/WBA lineage) | TrustZone part; verify the L5 RCC against RM0438 before reusing U5 offsets. |

## Tier C — new vendor (largest lift, largest new reach)

Each opens a vendor the engine has never modelled: new clock tree, new GPIO
conventions, new interrupt model, new SVD quirks.

| Candidate | Boards it unlocks | Core | Why it matters |
|-----------|-------------------|------|----------------|
| **CH32V003** | Countless ultra-cheap RISC-V boards | RV32EC | Opens **WCH**. Trendy and extremely cheap; note the core is RV32E**C** (16 registers), which the RISC-V decoder would need to handle. |
| **CC2538 / CC26xx** | Zigbee/Thread dev kits | Cortex-M3 / M4F | Opens **TI**, the last large vendor absent from the engine, without paying for a new ISA. |
| PSoC 6 / XMC | CY8CKIT-062, XMC dev kits | M4 + M0+ | Opens **Infineon**. |
| RA8M1 | Renesas EK-RA8M1 | Cortex-M85 | New core (Helium/MVE), but the RA family model from RA4M1 carries over. |
| MIMXRT1062 | Teensy 4.0 | M7 | The exact RT1062 part (the onboarded `imxrt1064` is the RT1064-class cousin); mostly config + fixture. |

## Explicitly out of scope

| Candidate | Why not |
|-----------|---------|
| MSP430 | New 16-bit ISA, not a new chip. TI reach is better served by CC2538 (Cortex-M3). |
| Anything Linux-class (STM32MP1, RPi 4) | Out of the deterministic-firmware-oracle problem entirely. |
| Radios bolted onto onboarded SoCs (CYW43439 on Pico W, ESP32-C6 WiFi) | Not MCUs — device/component layer, not `configs/chips/`. |

## Suggested order

1. **nRF54L05 / L10** — Tier A, siblings of an onboarded part; cheapest remaining
   add, and it extends a family already silicon-verified on its bigger siblings.
2. **STM32F446** — Tier A, popular board, no new models.
3. **ESP32-H2** — Tier B, rides the C6 work that just landed.
4. **CH32V003** — the first new vendor; unlocks the ultra-cheap RISC-V wave
   (pay the RV32EC decoder work once).
5. **CC2538** — opens TI without a new ISA.

## Honest caveat on "supported"

Onboarding a chip means it boots, runs firmware and passes the tier-1 matrix. It
does **not** mean silicon-verified — that needs a bench part and an SWD capture
(see `validation/boards/`). As of 2026-09-19 the manifest carries 34 board
entries: 9 silicon-tier (1 `silicon-smoke`, 8 `silicon-verified`), 9
`sim-validated`, 12 `smoke-manual`, 4 `structural`. The three parts in this batch
landed at `smoke-manual` (SIM-DERIVED, no bench capture), so plan onboarding and
validation as two separate pieces of work.
