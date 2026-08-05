# Pico 2 (RP2350) Onboarding Example

Run all commands from `core/`.

## Purpose

Deterministic bring-up for Raspberry Pi Pico 2 (RP2350, dual Cortex-M33,
2 MB XIP flash @ `0x10000000`, 520 KB SRAM @ `0x20000000`) reusing the
RP2040 behavioural models with the `rp2350` clock/reset profile (moved
APB map from rp2350 addressmap.h).

## Quick Run

```bash
rustup target add thumbv8m.main-none-eabi   # once
RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-rp2350-demo \
  --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/pico2/uart-smoke.yaml \
  --output-dir out/pico2/uart-smoke \
  --no-uart-stdout
```

Expected result:
1. smoke test passes (exit 0)
2. UART contains `RP2350_SMOKE_OK`

## Files

1. `system.yaml`: board mapping (LED on GP25 via SIO).
2. `uart-smoke.yaml`: deterministic UART oracle.
3. `REQUIRED_DOCS.md`: source-grounding references.
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
