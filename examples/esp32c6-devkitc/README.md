# ESP32-C6-DevKitC-1 Onboarding Example

Run all commands from `core/`.

## Purpose

Deterministic L1 bring-up for the **ESP32-C6** HP core (RV32IMAC @ 160 MHz) on
the **ESP32-C6-DevKitC-1-N8** using UART0 and the board's built-in WS2812B RGB
LED:

1. `pcr` — UART0 APB + function clock gates (the C6's replacement for the C3's
   SYSTEM/APB_CTRL blocks)
2. `io_mux` — firmware writes GPIO16's U0TXD selection; the model records this
   register value but does not enforce pad routing
3. `gpio` — C6 GPIO8 pad transitions from GPIO W1TS/W1TC writes
4. `rgb_led` — the WS2812 model decodes a bit-banged GRB frame into solid red
5. `uart0` — Espressif UART IP clocked at the C6's rate, `OK\n` to the console

The firmware prints `OK\n` through UART0 and sends a red WS2812 frame on GPIO8.
The smoke checks the UART output, GPIO edge count, and decoded framebuffer
colour. RMT is not modeled or used by this example. The USB-to-UART bridge and
electrical propagation through GPIO16/GPIO17 are outside the model.

**SIM-DERIVED.** Not silicon-verified. The LP core, ROM boot path, interrupt
matrix, every radio and every other peripheral are out of scope at this level —
see [`docs/boards/esp32c6-devkitc.md`](../../docs/boards/esp32c6-devkitc.md).

## Quick Run

```bash
cargo build -p firmware-esp32c6-demo --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c6-devkitc/io-smoke.yaml \
  --output-dir out/esp32c6-devkitc/io-smoke \
  --no-uart-stdout
```

Expected result:

1. all four smoke assertions pass
2. UART contains `OK`
3. the modeled WS2812 pixel decodes as red

## Files

1. [`configs/systems/esp32c6-devkitc.yaml`](../../configs/systems/esp32c6-devkitc.yaml):
   UART0 plus the on-board WS2812 on GPIO8.
2. `io-smoke.yaml`: deterministic UART, GPIO-edge, and decoded-pixel assertions.
3. `REQUIRED_DOCS.md`: source-grounding references (TRM, datasheet, DevKitC-1 guide, esp-idf headers, espressif/svd).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.

The committed [`evidence/`](evidence/) directory contains the build and local
Core CLI smoke logs, a generated onboarding scoreboard, vendor availability,
and the hosted-verifier limitation.
