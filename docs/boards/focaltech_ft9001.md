# Validation Report: focaltech_ft9001

**Architecture:** Arm Cortex-M4F (Renode `cortex-m4f`).

## 1. Dynamic Simulation Validation
**Status:** Onboarding smoke (`examples/renode-onboarding/focaltech_ft9001-smoke.yaml`,
firmware `crates/firmware-ft9001-smoke`). Evidence:
`docs/coverage/renode-onboarding/metrics/focaltech_ft9001/`.

The smoke proves only USART2's polled register path: TX gated on CR2.TE,
control-register read-back, FSR tracking the RX FIFO, and RXFCLR. USART2's
interrupt output, RX-enable gating, baud/framing behavior, and FSR2 error
flags are not modelled. It is local CLI simulation evidence, not hardware
confirmation or Renode equivalence.

## Hardware Coverage Report

Memory map from Renode `platforms/cpus/focaltech_ft9001.repl`:

| Region | Base | Size |
|---|---|---|
| `flash0` | `0x10000000` | 2 MiB |
| `sram0` | `0x20000000` | 224 KiB |
| `secure_rom` | `0x00080000` | 4 KiB (zero-filled; no ROM image) |
| `sramd0` | `0x1fff8000` | 32 KiB |

The import follows [Renode's FT9001 platform definition](https://github.com/renode/renode/blob/master/platforms/cpus/focaltech_ft9001.repl)
and [FT9001 USART model](https://github.com/renode/renode-infrastructure/blob/master/src/Emulator/Peripherals/Peripherals/UART/FT9001_USART.cs).

| Peripheral ID | Base Address | Type |
|---|---|---|
| `systick` | `0xe000e010` | `systick` |
| `usart2` | `0x40014000` | `ft9001_usart` (polled path; interrupts not modelled) |

**Total Peripherals:** 2

Not modelled and not declared: CPM, reset controller, TRNG, DWT, and the
blocks Renode itself only tags (pinctrl, CCM, flash controller, WDT, TC,
EPORT0-7, USART3, USB, I/D caches, MPU).
