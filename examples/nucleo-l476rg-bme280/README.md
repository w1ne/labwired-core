# NUCLEO-L476RG + Adafruit BME280

This example attaches Adafruit's BME280 breakout (PID 2652) to the existing
NUCLEO-L476RG model. It builds the Adafruit Arduino library firmware and checks
that the sensor answers its ID and calibration reads and returns a temperature
through the modeled I²C1 path.

The system binds the modeled sensor to I²C1 at `0x77`, which is the address
used by the firmware smoke. The LabWired pin model exposes I²C1 SCL on PB6 and
SDA on PB7; that is a model claim for the digital twin. The BME280 component
model exposes VCC, GND, SDA, SCL, CSB, and SDO. Power sequencing, the physical
breakout address strap, and its header routing are outside this simulation
evidence.

## Reproduce

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-l476rg-bme280 \
  --crate examples/nucleo-l476rg-bme280 \
  --target nucleo_l476rg \
  --firmware-build-script scripts/build_nucleo_l476rg_bme280_smoke.sh \
  --script examples/nucleo-l476rg-bme280/onboarding-smoke.yaml \
  --system configs/systems/nucleo-l476rg-bme280.yaml \
  --out-dir examples/nucleo-l476rg-bme280/evidence
```

The captured build and simulator output is in [`evidence/`](evidence/).
ST's board lifecycle and stock check and Adafruit's module stock check are in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The BME280 and STM32L476 I²C1 models already exist, so this adds a board
integration and a real library smoke rather than another sensor register-map
implementation. The smoke reads the BME280 ID, calibration block, and default
temperature, pressure, and humidity values. Conversion timing, filtering, and
oversampling dynamics are not modeled. The inherited full-chip descriptor also
declares SYSCFG as an explicit stub; this firmware does not access SYSCFG or
the board's other untested interfaces.
