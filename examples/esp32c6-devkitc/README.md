# ESP32-C6-DevKitC-1 Onboarding Example

Run all commands from `core/`.

## Purpose

Deterministic L1 bring-up for the **ESP32-C6** HP core (RV32IMAC @ 160 MHz) on
the **ESP32-C6-DevKitC-1-N8** using the modeled UART0 console path:

1. `pcr` — UART0 APB + function clock gates (the C6's replacement for the C3's
   SYSTEM/APB_CTRL blocks)
2. `io_mux` — firmware writes GPIO16's U0TXD selection; the model records this
   register value but does not enforce pad routing
3. `uart0` — Espressif UART IP clocked at the C6's rate, `OK\n` to the console

The firmware prints `OK\n` through UART0, mapped to the host console in the
twin. The smoke verifies the UART peripheral output. The USB-to-UART bridge and
electrical propagation through GPIO16/GPIO17 are outside the model.

**SIM-DERIVED.** Not silicon-verified. The LP core, ROM boot path, interrupt
matrix, every radio and every other peripheral are out of scope at this level —
see [`docs/boards/esp32c6-devkitc.md`](../../docs/boards/esp32c6-devkitc.md).

## Quick Run

```bash
cargo build -p firmware-esp32c6-demo --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c6-devkitc/uart-smoke.yaml \
  --output-dir out/esp32c6-devkitc/uart-smoke \
  --no-uart-stdout
```

Expected result:

1. smoke test passes
2. UART contains `OK`

## Files

1. [`configs/systems/esp32c6-devkitc.yaml`](../../configs/systems/esp32c6-devkitc.yaml):
   board profile limited to the UART0 console.
2. `uart-smoke.yaml`: deterministic UART smoke assertion.
3. `REQUIRED_DOCS.md`: source-grounding references (TRM, datasheet, DevKitC-1 guide, esp-idf headers, espressif/svd).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.

The committed [`evidence/`](evidence/) directory contains the build and local
Core CLI smoke logs, a generated onboarding scoreboard, vendor availability,
and the hosted-verifier limitation.
