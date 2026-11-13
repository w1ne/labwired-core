# Adafruit Metro M4 Onboarding Example

Run all commands from `core/`.

## Purpose

This example provides deterministic bring-up for Adafruit Metro M4 Express (SAMD51J19A) using the supported subset:
1. `mclk` / `gclk` (SAMD51 clock gating — MCLK masks + GCLK PCHCTRL)
2. `porta` / `portb` (`sam_port`)
3. `sercom3` UART (Serial1 on PA22/PA23)
4. `systick`

The mixed-signal fixture routes a generic Beta NTC divider to A0 and an unrelated
Shockley diode to A4, then checks both through ADC0 conversions.
`analog-mosfet-junction-density.yaml` adds three forward-biased level-1 NMOS
bulk junctions on A1/A2/A3: `JS` times symmetric `AD`/`AS`, `JS` times
asymmetric `AD`/`AS`, and the ngspice fallback to `IS` when `AS` is absent.
This is simulation-derived coverage, not silicon validation. Run its end-to-end proof with:

```bash
cargo test -p labwired-core --test cosim_metro_m4_analog
```

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
