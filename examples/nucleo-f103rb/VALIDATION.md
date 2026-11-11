# Validation — NUCLEO-F103RB

**Status: simulator smoke passed 2026-10-27.** This validates the modeled
STM32F103RBT6 system and firmware path. It is not a physical-board test.

## Build and smoke results

Tool: `labwired-cli 0.25.0`

Firmware target: `thumbv7m-none-eabi`

Firmware SHA-256: `8d3407f85e744978a678eddce3fb427b057c244ca91d9a4d2e16b57270e33630`

```sh
cargo build -p firmware-nucleo-f103rb-smoke --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-f103rb/io-smoke.yaml \
  --output-dir out/nucleo-f103rb/io-smoke --no-uart-stdout
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-f103rb/uart-smoke.yaml \
  --output-dir out/nucleo-f103rb/uart-smoke --no-uart-stdout
```

- Firmware build completed successfully.
- GPIO + USART2 smoke: **PASS, 3/3 assertions**, 200,000 steps. UART captured
  `OK`; PA5 captured **10 edges**; stop reason was `MaxSteps`.
- UART-only smoke: **PASS, 2/2 assertions**, 200,000 steps. UART captured
  `OK`; stop reason was `MaxSteps`.

## Mixed-signal ADC path

The separate [`analog-ntc-system.yaml`](analog-ntc-system.yaml) manifest routes
a temperature-driven NTC divider through the in-core MNA solver to
`board.analog.pa0_volts`. The board-specific test resolves PA0 through the
STM32F103RB descriptor and checks the firmware-visible ADC1 data register at
cold, nominal, and hot temperatures.

```sh
cargo test -p labwired-core --test cosim_pin_routing \
  nucleo_f103rb_ntc_netlist_reaches_pa0_adc1_end_to_end -- --nocapture
cargo test -p labwired-core --test cosim_pin_routing \
  nucleo_f103rb_diode_depletion_and_transit_charge_reach_adc1_end_to_end -- --nocapture
cargo test -p labwired-core --test cosim_pin_routing \
  nucleo_f103rb_mosfet_gate_drain_overlap_charge_reaches_adc1_end_to_end -- --nocapture
```

The second test routes matched CJO/TT diode branches onto PA2–PA5/ADC1_IN2–IN5,
asserts each first transient against its no-charge branch, checks firmware-
visible ADC conversions, and verifies the branches converge to an independent
Shockley DC load line. The third test routes matched generic NMOS branches with
and without CGDO overlap to PA2/PA3/ADC1_IN2–IN3 alongside the unrelated NTC
and diode; it checks the independent backward-Euler RC recurrence, ADC
readback, and undriven neighboring channel. These are simulator proofs for
NUCLEO-F103RB and do not claim a physical board measurement.

## Unsupported-instruction audit

```sh
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/firmware-nucleo-f103rb-smoke \
  --system configs/systems/nucleo-f103rb.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo-f103rb \
  --fail-on-unsupported
```

Result: simulator exit 0; **200,000 instructions executed, 0 unsupported**
(100% instruction support for this run). The generated report was
`out/unsupported-audit/nucleo-f103rb/report.md`.

Physical NUCLEO-F103RB UART/GPIO comparison remains unverified. Peripheral
coverage beyond the documented GPIO and USART2 smoke path remains subject to
the limits in `KNOWN_LIMITATIONS.md`.
