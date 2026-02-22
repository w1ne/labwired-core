# NUCLEO-F401RE Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for NUCLEO-F401RE using the minimal supported subset:
1. `rcc`
2. `gpio`
3. `uart`
4. `systick`

## Quick Run

```bash
cargo build -p firmware-f401-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nucleo-f401re/uart-smoke.yaml --output-dir out/nucleo-f401re/uart-smoke --no-uart-stdout
```

Expected result:
1. smoke test passes
2. UART contains `OK`

## Files

1. `system.yaml`: local board mapping for simulation runs.
2. `uart-smoke.yaml`: deterministic UART smoke assertion.
3. `io-smoke.yaml`: strict onboarding smoke path.
4. `REQUIRED_DOCS.md`: source-grounding references.
5. `EXTERNAL_COMPONENTS.md`: external component declaration.
6. `VALIDATION.md`: reproducible validation/audit commands.
