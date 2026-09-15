# Adafruit Metro M4 Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for Adafruit Metro M4 Express (SAMD51J19A) using the minimal supported subset:
1. `mclk` / `gclk` (SAMD51 clock gating — MCLK masks + GCLK PCHCTRL)
2. `porta` / `portb` (`sam_port`)
3. `sercom3` UART (Serial1 on PA22/PA23)
4. `systick`

On-board QSPI flash, USB, and NeoPixel are intentionally omitted.

**SIM-DERIVED.** Not silicon-verified. Bench silicon may be ItsyBitsy M4 (SAMD51G19A); Playground pinout is Metro M4.

## Quick Run

```bash
cargo build -p firmware-atsamd51-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/metro-m4/uart-smoke.yaml --output-dir out/metro-m4/uart-smoke --no-uart-stdout
```

Expected result:
1. smoke test passes
2. UART contains `OK`

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `uart-smoke.yaml`: deterministic UART smoke assertion.
3. `REQUIRED_DOCS.md`: source-grounding references (DS60001507, pinout).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.
