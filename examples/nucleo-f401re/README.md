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

[`analog-jfet-terminal-resistance.yaml`](analog-jfet-terminal-resistance.yaml) adds a
generic level-1 NJF load line on PA2–PA5/ADC1_IN2–IN5 beside the unrelated diode on
PA0/ADC1_IN0. The co-simulation test checks RD and RS independently against scalar
load-line roots and reads each route through the firmware-visible ADC1 data register:

```sh
cargo test -p labwired-core --test cosim_nucleo_f401_analog nucleo_f401re_jfet_terminal_resistances_reach_adc1_with_unrelated_diode
```

[`analog-mosfet-body-effect.yaml`](analog-mosfet-body-effect.yaml) routes a matched
generic level-1 NMOS pair through PA2/PA3 (ADC1_IN2/IN3), beside the same independent
Shockley diode on PA0/ADC1_IN0. The board test compares the solved load lines against an
independent GAMMA/PHI threshold calculation while sweeping reverse bulk bias, then checks
the firmware-visible ADC data register on all three channels:

```sh
cargo test -p labwired-core --test cosim_nucleo_f401_analog nucleo_f401re_mosfet_body_effect_reaches_adc1_with_unrelated_diode
```
