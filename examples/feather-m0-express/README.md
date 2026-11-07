# Adafruit Feather M0 Express

This narrow profile boots ATSAMD21G18A firmware and toggles the board's red
D13 LED on PA17. The smoke enables the SAMD21 PORT clock through
`PM.APBBMASK`, configures PA17 as an output, and requires at least 16 observed
GPIO edges.

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

Build and simulation logs are committed in [`evidence/`](evidence/). The live
board and MCU availability check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The profile exposes the Cortex-M0+ core, flash/RAM geometry, SAMD21 PM
`APBBMASK`, and PORT group A with its PORT clock gate. It exercises only the
onboard red LED: Arduino D13 / PA17.

The NeoPixel on PA06, external 2 MB SPI flash, native USB bootloader/CDC,
SERCOM UART/I²C/SPI, ADC, DAC, timers, battery charger, and charging indicator
are outside this smoke profile and are not claimed as modeled board paths.
The firmware leaves the MCU at its 1 MHz reset clock and does not configure
the board's 48 MHz DFLL clock.
