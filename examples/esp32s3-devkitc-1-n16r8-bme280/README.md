# ESP32-S3-DevKitC-1-N16R8 + BME280

**Demand rank:** ESP32 is #1 in the measured target list (38% of respondents).
This is board-depth work on the already-modelled ESP32-S3, not a new MCU. The
DevKitC-1 is Espressif's breadboard-friendly carrier and has a real I2C sensor
read in the smoke.

## Modelled smoke

`configs/systems/esp32s3-devkitc-1-n16r8-bme280.yaml` describes the
ESP32-S3-DevKitC-1 with its N16R8 module and an Adafruit BME280 STEMMA QT
breakout. The firmware uses I2C0 on SDA=GPIO8 and SCL=GPIO9, reads the BME280
chip ID (`0xD0` → `0x60`) and raw-temperature registers (`0xFA`–`0xFC`), and
prints the observed values on UART0. The accepted model output is:

```text
ESP32S3_DEVKITC1_N16R8_BME280_OK ID=0x60 TEMP_RAW=0x082DC0 I2C0=GPIO8/GPIO9
```

Live availability evidence is recorded in [vendor-status.md](vendor-status.md).
The board remains in Espressif's live product catalog; Adafruit product 2652
was in stock when checked on 2026-11-10.

## Run

From the repository root:

```sh
examples/esp32s3-devkitc-1-n16r8-bme280/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32s3-devkitc-1-n16r8-bme280/bme280-smoke.yaml \
  --output-dir out/esp32s3-devkitc-1-n16r8-bme280 \
  --no-uart-stdout
```

The test reads the attached BME280 model through I2C0 and requires the exact
ID and raw-temperature sample. The checked-in `firmware/src/main.cpp` is the
copy-paste Arduino example for the board and breakout.

## Wiring

| BME280 STEMMA QT breakout | ESP32-S3-DevKitC-1-N16R8 |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | GPIO8 |
| SCL | GPIO9 |

## Coverage limits

- The firmware uses the 16 MB flash image and internal SRAM. The S3 model
  provisions the module's 8 MB PSRAM, but this example does not access it.
- Wi-Fi and Bluetooth use firmware thunks; eFuse and RTC_CNTL are limited
  stubs. Board buttons, the addressable LED, and USB host/device behavior are
  outside this profile's smoke.
- The I2C controller and BME280 register behavior are exercised. The simulator
  does not prove physical pull-ups, voltage levels, board wiring, or silicon
  parity.

See [VALIDATION.md](VALIDATION.md) for the recorded run and [EXTERNAL_COMPONENTS.md](EXTERNAL_COMPONENTS.md)
for the peripheral-model boundary.
