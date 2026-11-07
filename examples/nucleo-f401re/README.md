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

## Mixed-signal ADC example

[`analog-diode-system.yaml`](analog-diode-system.yaml) solves a diode load line
with the in-core MNA engine and routes its node voltage through PA0 to ADC1.
The board-level assertion compares against an independent Shockley load-line
reference, checks the selected and adjacent ADC channels, and asserts the
ADC1 conversion result read from its data register:

```sh
cargo test -p labwired-core --test cosim_pin_routing nucleo_f401re_diode_netlist_reaches_pa0_adc1_end_to_end
```
