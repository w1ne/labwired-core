# Vendored vendor SVDs (`tests/fixtures/real_world/`)

These files are consumed by `crates/core/tests/register_coverage.rs`, which
enumerates every register the vendor declares and probes the simulator's bus for
it. A missing or incomplete SVD therefore does not merely reduce coverage — it
**silently disarms the gate for whatever it fails to declare**, because the scan
cannot fail on a register the SVD never mentions.

New entries record their provenance here, following the convention in
`tests/fixtures/svd/README.md`. Files that predate this README do not yet have a
record; add one when you next touch them.

## stm32f411.svd

**Source:** [`modm-io/cmsis-svd-stm32`](https://github.com/modm-io/cmsis-svd-stm32)
→ `stm32f4/STM32F411.svd` (device `STM32F411`, version 1.9, 856 KB)
**Vendored:** 2026-07-27, for the STM32F411CEU6 (WeAct Black Pill) onboarding.
**Consumed by:** `register_coverage.rs` (chip `stm32f411ceu6`).

### Why modm-io and not cmsis-svd/cmsis-svd-data

Two public F411 SVDs exist and they are **not** equivalent. Measured on the
actual files:

| | modm-io (vendored) | cmsis-svd-data (rejected) |
|---|---|---|
| Peripherals | 47 | 55 |
| **Declared interrupts** | **56 (max 85)** | **34 (max 84)** |
| SPI5 peripheral | yes | yes |
| SPI5 interrupt | **yes (85)** | **absent** |
| USART1/2/6 interrupts | yes (37/38/71) | absent |
| TIM4 / TIM5 interrupts | yes (30/50) | absent |
| DMA stream interrupts | yes | absent |

The cmsis-svd-data file's NVIC table omits USART1/2/6, TIM4, TIM5, every DMA
stream **and SPI5** — precisely the peripheral this onboarding adds. Vendoring
it would install a gate that looks armed and verifies nothing about any of them.
For scale, the in-tree `stm32f401.svd` declares 54 interrupts up to 84, so the
modm file is of comparable quality and the cmsis-svd one is not.

### Residual gap — read this before trusting the gate

**Neither** F411 SVD declares `RCC.APB2ENR.SPI5EN`. The string `SPI5EN` does not
occur anywhere in the vendored file. That bit position (20) is taken from ST's
own CMSIS header
([`STMicroelectronics/cmsis_device_f4`](https://github.com/STMicroelectronics/cmsis_device_f4),
`Include/stm32f411xe.h`, `RCC_APB2ENR_SPI5EN_Pos`) and **cannot be cross-checked
against any SVD**. It is the one value in `configs/chips/stm32f411ceu6.yaml`
with a single source. The tier-1 fixture's `spi` check exercises it (gate off →
CR1 write dropped; gate on → transfer runs), so it is at least executable
evidence rather than an unchecked constant.

Everything else in the chip descriptor that the SVD *does* declare — SPI5's base
`0x40015000` and its IRQ 85, the RCC enable-register offsets, the GPIO port set
A/B/C/D/E/H — agrees between the SVD and the header.

Do not edit these files by hand. To refresh, re-download from the upstream repo
and re-check the interrupt count before committing.
