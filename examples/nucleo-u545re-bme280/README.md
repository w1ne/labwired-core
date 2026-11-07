# NUCLEO-U545RE-Q + Adafruit BME280

This example reads the BME280 chip ID over the Nucleo's Arduino I2C1 pins and
blinks LD2 while reporting the result on the ST-LINK virtual COM port. The
system uses the real `stm32u545` I2C1 model and the LabWired BME280 model at
address `0x77`.

Demand context: STM32 Nucleo/Discovery boards rank #5 in the kit survey (23%).
This is a coverage expansion for an existing U545 board model, not a new MCU
family.

## Wiring

| Adafruit BME280 STEMMA QT breakout | NUCLEO-U545RE-Q |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | D14 / PB7 |
| SCL | D15 / PB6 |

The breakout's default address is `0x77`. The firmware reads register `0xD0`
and requires the model's chip ID `0x60`; it also toggles the on-board green
LD2 on PA5. The simulator provides the sensor's deterministic register
response. It does not model environmental conversion dynamics or physical
sensor tolerances.

## Run

From the repository root:

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-u545re-bme280 \
  --crate firmware-nucleo-u545re-smoke \
  --target thumbv8m.main-none-eabi \
  --firmware-build-script scripts/build_nucleo_u545re_bme280_smoke.sh \
  --script examples/nucleo-u545re-bme280/onboarding-smoke.yaml \
  --system configs/systems/nucleo-u545re-bme280.yaml \
  --out-dir examples/nucleo-u545re-bme280/evidence
```

Live board, MCU, and breakout availability is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md). This is simulator
evidence, not a physical hardware capture.
