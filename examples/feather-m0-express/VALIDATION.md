# Feather M0 Express validation

## Smoke scope

The committed local CLI smoke builds bare-metal Cortex-M0+ firmware for the
ATSAMD21G18A, enables `PM.APBBMASK.PORT_`, configures PA17, and toggles the
board's D13 red LED. It asserts at least 16 GPIO edges. The focused chip
profile includes only the PM clock-mask model and PORT group A, with the PORT
clock gate enforced by the simulator.

## Reproduce

```bash
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id feather-m0-express \
  --crate firmware-feather-m0-express-smoke \
  --target thumbv6m-none-eabi \
  --script examples/feather-m0-express/io-smoke.yaml \
  --system configs/systems/feather-m0-express.yaml \
  --out-dir examples/feather-m0-express/evidence
```

The exact build and run logs are in `evidence/logs/`; the simulator result is
in `evidence/simulation-output/`. No physical Feather was connected, so this
is local simulator evidence rather than a hardware capture. The hosted
LabWired catalog has no Feather M0 board target; its Arduino Zero entry for
the same ATSAMD21G18A reports `compilable: false`. This record therefore does
not claim a hosted `model_verified` verdict. The vendor pages and observed
availability are recorded in `evidence/vendor-status.md`.

## Deliberate limits

This profile does not claim USB, USB CDC, the NeoPixel, external SPI flash,
SERCOM, ADC/DAC, timers, battery charging, or other MCU peripherals. The
firmware does not configure the 48 MHz DFLL clock; its configured simulated
reset clock remains 1 MHz.
