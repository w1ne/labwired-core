# STM32U575I-EV — smoke example

The **STM32U575I-EV** is ST's evaluation board (MB1550) for the
**STM32U575AII6Q** (Arm Cortex-M33, 2 MiB flash, 768 KiB main SRAM + 16 KiB
SRAM4). It shares the U5 register map with the NUCLEO-U575ZI-Q, so both boards
use the same chip descriptor (`configs/chips/stm32u575.yaml`).

This example runs a bare-metal firmware that:

1. enables the GPIOB/GPIOH port clocks and USART1 via RCC;
2. configures **LED5 (PB7)** and **LED6 (PH7)** as active-high outputs;
3. drives each LED independently and prints the **ODR value read back** from
   the GPIO block after every write;
4. prints a heartbeat on **USART1** (COM1 PA9/PA10, the ST-LINK-V3E VCP).

Ledger: **L1 smoke, SIM-DERIVED** — real firmware runs in-sim, but there is no
bench part and no silicon capture. The 60North `renode-stm32u5` package is a
Renode-derived reference, not silicon (see
[`../../validation/stm32u5-60north/README.md`](../../validation/stm32u5-60north/README.md)).

## Build

```bash
rustup target add thumbv8m.main-none-eabi   # once
cargo build -p firmware-stm32u575i-ev-smoke --release --target thumbv8m.main-none-eabi
```

## Run

```bash
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-stm32u575i-ev-smoke \
  --system examples/stm32u575i-ev/system.yaml --max-steps 4000000
```

Expected console output (prefix):

```text
STM32U575I-EV SMOKE
cpu=cortex-m33 uart=usart1
leds=off odr_b=00000000 odr_h=00000000
led5=on odr_b=00000080 odr_h=00000000
led6=on odr_b=00000080 odr_h=00000080
led5=off odr_b=00000000 odr_h=00000080
led6=off odr_b=00000000 odr_h=00000000
hb=0 odr_b=00000080 odr_h=00000080
...
```

`odr_b`/`odr_h` are the GPIOB/GPIOH output-data registers read from the model
(LED5 = PB7 = bit 7, LED6 = PH7 = bit 7).

## Smoke test

```bash
./target/release/labwired test \
  --script examples/stm32u575i-ev/io-smoke.yaml \
  --output-dir out/stm32u575i-ev-smoke --no-uart-stdout
# PASS  3/3 checks · io-smoke · 4000000 steps
```

The example uses the committed ELF
`tests/fixtures/stm32u575i-ev-smoke.elf`, so it runs without a cross toolchain.

## Pins (from UM2854 / ST BSP v1.4.1)

| Board label | MCU pin | Notes |
|-------------|---------|-------|
| LED5 (green) | PB7 | active high (`BSP_LED_On` drives high); `BSP_LED_GREEN` |
| LED6 (red) | PH7 | active high; `BSP_LED_RED` |
| LED7 (blue) | MFX I/O expander | not a direct GPIO — not modelled |
| LED8 (yellow) | MFX I/O expander | not a direct GPIO — not modelled |
| USER button | PC13 | active high, external pull-down |
| COM1 TX/RX | PA9 / PA10 | USART1 AF7, 115200 8N1 (ST-LINK-V3E VCP) |

## What the chip models

The chip descriptor is shared with the NUCLEO-U575ZI-Q; see
[`../../docs/boards/stm32u575.md`](../../docs/boards/stm32u575.md) for the full
model/honesty table. The surfaces this smoke drives are RCC clock gates, the
`stm32v2` GPIO output path, USART1 TX, and the U5 memory map.

## Documents

- [`VALIDATION.md`](VALIDATION.md) — claims, evidence, and explicit non-claims
- [`REQUIRED_DOCS.md`](REQUIRED_DOCS.md) — board/MCU sources
- [`EXTERNAL_COMPONENTS.md`](EXTERNAL_COMPONENTS.md) — what is not modelled
- [`../../docs/boards/stm32u575i-ev.md`](../../docs/boards/stm32u575i-ev.md) — board page
