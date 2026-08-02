# Onboarding candidates

Which chips are worth adding next, and what each actually costs given the models
already in the tree. Ordered by cost, not by preference — a Tier A part is often
a weekend's work reusing shipped models, while a Tier C part means a new vendor
family from scratch.

Companion to [`board_onboarding_playbook.md`](board_onboarding_playbook.md),
which describes *how* to onboard. This file is *what* to onboard.

## What the engine already supports

Onboarding cost is dominated by how much of this a candidate can reuse.

| Axis | Supported today |
|------|-----------------|
| Cores | Cortex-M0+, M3, M4, M7, M33; RISC-V (RV32IMC); Xtensa LX6 / LX7 |
| STM32 RCC families | F1, F4, V2 (H5/WBA), H5, H7, L4, L0, G4, WB |
| Vendors | ST, Nordic, Espressif, Raspberry Pi, NXP (Kinetis MKW41Z) |
| **Absent vendors** | **Microchip/Atmel, Renesas, WCH, TI, Silabs, Infineon** |

A candidate whose core is already supported and whose RCC/clock tree matches an
existing family is cheap. A candidate that needs a new core *or* a new vendor
clock tree is not.

## Tier A — near-free (reuses a shipped family)

These need a chip config, a tier-1 fixture and a validation entry. No new
peripheral models, or close to none.

| Candidate | Board it unlocks | Reuses | Notes |
|-----------|------------------|--------|-------|
| **STM32F411** | **WeAct Black Pill** | F4 RCC/GPIO/timer/SPI/I²C/ADC — all shipping via F401/F407 | Highest reach-per-effort on this list. F401 sibling; differs mainly in RAM/flash size and clock ceiling. |
| STM32G431 | NUCLEO-G431RB | G4 family (G474 shipped, RM0440 offsets already correct) | Cheapest STM32 in the modern line. |
| STM32L432 / L452 | NUCLEO-L432KC, Feather STM32L4 | L4 family (L476 is silicon-verified) | |
| STM32F446 | NUCLEO-F446RE | F4 family | |
| STM32H723 / H725 / H730 | NUCLEO-H723ZG | H7 family (H735 shipped, RM0468) | Same RCC layout; H735 work carries over directly. |
| STM32L071 / L031 | NUCLEO-L031K6 | L0 family (L073 silicon-verified) | |
| nRF52833 | micro:bit v2 | nRF52 family (52832/52840 both silicon-verified) | micro:bit is a large education install base. |
| nRF54L05 / L10 | — | nRF54L15 already onboarded; siblings differ in memory size | SVDs already fetched for all three. |

## Tier B — moderate (new peripherals, existing core + arch)

New register sets to model, but no new CPU and no new vendor conventions.

| Candidate | Board it unlocks | Reuses | New work |
|-----------|------------------|--------|----------|
| **RP2350** | **Raspberry Pi Pico 2** | Cortex-M33 (proven by WBA52); most RP2040 peripherals carry over | PIO v2, PowerMan, OTP/security. Pairs naturally with any RP2040 peripheral work — do RP2040 ADC/PWM/RTC first and much of this is free. |
| **STM32U5** | NUCLEO-U575ZI-Q | M33; the V2 RCC layout is **U5-derived** — WBA52 already uses those exact offsets (AHB2ENR@0x8C / APB1ENR1@0x9C / APB2ENR@0xA4) | Mostly config + fixture; ST's current flagship low-power line. |
| **STM32G0** | NUCLEO-G071RB | M0+ | New G0 RCC layout. Very high volume — ST's cheap workhorse. |
| STM32C0 | NUCLEO-C031C6 | M0+ | ST's cheapest new line; small peripheral set makes it a fast add. |
| ESP32-C6 | ESP32-C6-DevKitC | RISC-V core + much of the C3 peripheral map | WiFi 6 + 802.15.4. Current-generation Espressif part. |
| ESP32-H2 | ESP32-H2-DevKitM | RISC-V + C6-adjacent map | Thread/Zigbee focus. |
| ESP32-S2 | ESP32-S2 Saola | Xtensa LX7 (S3 shipped) | Single-core S3 sibling. |

## Tier C — new vendor (largest lift, largest new reach)

Each opens a vendor the engine has never modelled: new clock tree, new GPIO
conventions, new interrupt model, new SVD quirks.

| Candidate | Boards it unlocks | Core | Why it matters |
|-----------|-------------------|------|----------------|
| **SAMD21** | Arduino Zero, Adafruit Feather M0, QT Py, Trinket M0 | M0+ | Opens **Microchip**, currently absent. Enormous maker install base; the default "small Adafruit board" silicon. |
| SAMD51 | Adafruit Feather M4, Metro M4, Grand Central | M4F | Follows SAMD21 at a fraction of the cost once the SAM family exists. |
| **RA4M1** | **Arduino UNO R4 Minima / WiFi** | M4 | Opens **Renesas**. The current flagship Arduino board — arguably the single highest-reach board not supported. |
| CH32V003 | Countless ultra-cheap RISC-V boards | RV32EC | Opens **WCH**. Trendy and extremely cheap; note the core is RV32E**C** (16 registers), which the RISC-V decoder would need to handle. |
| i.MX RT1062 | Teensy 4.0 / 4.1 | M7 | M7 already proven by H735. Strong audio/high-performance maker niche. |

## Explicitly out of scope

| Candidate | Why not |
|-----------|---------|
| ATmega328P (Arduino UNO R3) | AVR 8-bit — no AVR core in the engine. A new ISA, not a new chip. |
| MSP430 | Same reason: new 16-bit ISA. |
| Pico W's CYW43439 | Not an MCU — a separate WiFi part over SPI. Belongs in the device/component layer, not `configs/chips/`. |
| Anything Linux-class (STM32MP1, RPi 4) | Out of the deterministic-firmware-oracle problem entirely. |

## Suggested order

1. **STM32F411** — Tier A, largest reach-per-hour on the list.
2. **RP2040 peripheral depth** (ADC/PWM/RTC/watchdog) — not an onboarding, but it
   raises the weakest-covered popular part in the fleet and is the prerequisite
   that makes **RP2350** cheap.
3. **RP2350** — inherits step 2.
4. **SAMD21** — the first new vendor; unlocks the whole Adafruit M0 line.
5. **RA4M1** — highest single-board reach, but pay for it after the SAM family
   has proven the new-vendor path.

## Honest caveat on "supported"

Onboarding a chip means it boots, runs firmware and passes the tier-1 matrix. It
does **not** mean silicon-verified — that needs a bench part and an SWD capture
(see `validation/manifest.yaml`). Of the parts shipped today, only 9 of 15 boards
carry a live silicon capture. A new Tier B/C part will land at `structural` or
`sim-validated` and stay there until someone runs the hardware diff, so plan
onboarding and validation as two separate pieces of work.
