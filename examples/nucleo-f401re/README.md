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
7. `analog-ntc-diode-system.yaml`: generic Beta NTC and Shockley diode decks on
   the in-core analog engine.
8. `analog-ntc-diode-ngspice.yaml`: the same decks and routes on `adapter:
   ngspice`, with `ntc-beta.cir` and `diode-load-line.cir`.

## Analog reference solver

The two analog decks run on either engine without changing names, routes or
parameters. ngspice needs `python3` and `libngspice`:

```bash
labwired cosim-step examples/nucleo-f401re/analog-ntc-diode-ngspice.yaml \
  --set ui.ntc.temperature_c=25 --steps 4 --json
```

The diode deck pins ngspice's temperature to the in-core engine's 300.00 K.
`crates/cli/tests/analog_vs_ngspice_differential.rs` compares both engines at
0, 25 and 60 °C.
