# Validation record

## Demand and selection

ESP32 is demand rank **#1** in the JetBrains State of Embedded 2025 survey
(38% of reported MCU use). This round adds an I2C sensor exercise to the
already-described ESP32-DevKitC-32E system. It is incremental ESP32 family
depth, not a new MCU or a new vendor bet.

## Smoke scope

The PlatformIO Arduino firmware routes I2C0 to GPIO21/GPIO22, reads BME280
register `0xD0` and requires ID `0x60`, then reads `0xFA`–`0xFC` and requires raw
temperature `0x082DC0`. Only after those checks does it toggle GPIO23. The
committed smoke requires at least eight GPIO23 edges and records the complete
build and simulator result under `evidence/`.

The system explicitly attaches an Adafruit BME280 model at I2C address `0x77`.
The classic ESP32 system builder also currently installs its legacy BMP280
fixture at `0x76`; this test does not address or depend on that second device.
UART output, Wi-Fi/Bluetooth, flash/ROM boot fidelity, and physical-board
confirmation are outside this smoke. GPIO23 edges are model observations, not
package-voltage measurements.

## Reproduce

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e-bme280 \
  --firmware-build-script examples/esp32-devkitc-32e-bme280/firmware/build-firmware.sh \
  --script examples/esp32-devkitc-32e-bme280/bme280-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-bme280.yaml \
  --out-dir examples/esp32-devkitc-32e-bme280/evidence \
  --threshold-seconds 3600
```

For a physical setup, connect the BME280 breakout's 3V and GND to the board's
3V3 and GND, SDA to GPIO21 (J3 pin 6), and SCL to GPIO22 (J3 pin 3). GPIO23
(J3 pin 2) can drive an external LED through a current-limiting resistor.

This local CLI smoke is simulator evidence, not hardware confirmation or the
LabWired hosted `labwired_verify` oracle verdict.
