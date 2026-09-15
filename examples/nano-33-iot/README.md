# Arduino Nano 33 IoT Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for Arduino Nano 33 IoT (SAMD21G18A) using the minimal supported subset:
1. `pm` / `gclk` (SAM clock gating)
2. `porta` / `portb` (`sam_port`)
3. `sercom5` UART (Serial1 on PB22/PB23)
4. `systick`

On-board NINA (WiFi), ATECC608A, and IMU are intentionally omitted.

## Quick Run

```bash
cargo build -p firmware-atsamd21-demo --release --target thumbv6m-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nano-33-iot/uart-smoke.yaml --output-dir out/nano-33-iot/uart-smoke --no-uart-stdout
```

Expected result:
1. smoke test passes
2. UART contains `OK`

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `uart-smoke.yaml`: deterministic UART smoke assertion.
3. `REQUIRED_DOCS.md`: source-grounding references (DS40001882, pinout).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.
