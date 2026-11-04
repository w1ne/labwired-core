# NUCLEO-F411RE + Adafruit BME280

This system combines the ST NUCLEO-F411RE with Adafruit's currently stocked
BME280 STEMMA QT breakout (PID 2652). The smoke runs Adafruit's Arduino library,
probes the sensor over the board variant's default `Wire` connection, and reads
temperature, pressure, and humidity through the modeled I2C1 bus.

## Wiring

| BME280 breakout | NUCLEO-F411RE |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | D14 / PB9 |
| SCL | D15 / PB8 |

`Wire.begin()` uses I2C1 on D14/D15 (PB9/PB8) in the NUCLEO-F411RE Arduino
variant. The system descriptor attaches the BME280 model at `0x77`. The Nucleo's ST-LINK
VCP path is USART2 on PA2/PA3 and captures the sensor result.

## Reproduce

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-f411re-bme280 \
  --crate examples/nucleo-f411re-bme280 \
  --firmware-build-script scripts/build_nucleo_f411re_bme280_smoke.sh \
  --script examples/nucleo-f411re-bme280/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f411re-bme280.yaml \
  --out-dir examples/nucleo-f411re-bme280/evidence
```

The committed build and simulator logs are in [`evidence/`](evidence/). Live
vendor status is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The focused chip profile exposes RCC, flash control, PWR, GPIOA, GPIOB, SysTick,
USART2, and I2C1. The BME280 model answers the ID and calibration reads and returns its deterministic default
measurements. It does not simulate sensor conversion dynamics, oversampling,
filtering, or physical power sequencing. Other F411RE peripherals are omitted
and unsupported by this focused profile. The smoke is simulator evidence, not
hardware confirmation.
