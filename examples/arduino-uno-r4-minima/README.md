# Arduino Uno R4 Minima Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for Arduino Uno R4 Minima (R7FA4M1AB) using the minimal supported subset:
1. `ra_sysc` (HOCO / OSCSF)
2. `port1` / `port3` (`ra_port`)
3. `sci2` UART (header Serial on P301/P302)
4. `systick`

USB CDC Serial is intentionally out of scope (USBFS is a stub window only).

**SIM-DERIVED.** Not silicon-verified. Playground bring-up is later.

## Quick Run

```bash
cargo build -p firmware-ra4m1-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/arduino-uno-r4-minima/uart-smoke.yaml --output-dir out/arduino-uno-r4-minima/uart-smoke --no-uart-stdout
```

Expected result:
1. smoke test passes
2. UART contains `OK`

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `uart-smoke.yaml`: deterministic UART smoke assertion.
3. `REQUIRED_DOCS.md`: source-grounding references (R01UH0887, pinout; FSP PAC as address cross-check only).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.
