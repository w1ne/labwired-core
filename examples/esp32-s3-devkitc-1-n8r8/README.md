# ESP32-S3-DevKitC-1-N8R8 + BME280

## Demand position

ESP32 is the survey's **#1 MCU family** (38% of respondents; ESP32 boards are
42% of kits). This is a board-depth pick within that demand rank. The N8R8 is
Espressif's currently listed DevKitC-1 variant; it fills the current 8 MB
flash / 8 MB PSRAM offering that the older N16R8 descriptor does not describe.
See the live [vendor availability receipt](evidence/bme280/vendor-status.md).

## Wiring

Use an Adafruit BME280 STEMMA QT breakout at I2C address `0x77`:

| BME280 breakout | ESP32-S3-DevKitC-1-N8R8 |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | GPIO8 |
| SCL | GPIO9 |

The smoke config routes I2C0 through the GPIO matrix on GPIO8 and GPIO9. It
reads the BME280 ID (`0x60`) and a modeled raw-temperature register sample over
I2C. UART0 output is captured by the local simulator.

## Copy-paste Arduino example

Install Adafruit's `Adafruit BME280 Library` and `Adafruit Unified Sensor`
libraries, connect the breakout as shown above, then upload this sketch:

```cpp
#include <Adafruit_BME280.h>
#include <Arduino.h>
#include <Wire.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin(8, 9, 100000);  // SDA=GPIO8, SCL=GPIO9
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    return;
  }
  Serial.printf("BME280_ID=0x%02X\n", bme.sensorID());
  Serial.printf("Temperature C: %.2f\n", bme.readTemperature());
}

void loop() {}
```

The hosted Arduino verifier reached the BME280 ID marker on the catalog's
generic DevKitC-1 model with a 100,000,000-step budget. The catalog does not
yet expose an exact N8R8 target; the separate local smoke exercises the
N8R8-specific flash descriptor. See
[`evidence/bme280/hosted-verification.md`](evidence/bme280/hosted-verification.md).

## Reproduce the local simulator smoke

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-s3-devkitc-1-n8r8-bme280 \
  --script examples/esp32-s3-devkitc-1-n8r8/bme280-smoke.yaml \
  --system configs/systems/esp32-s3-devkitc-1-n8r8-bme280.yaml \
  --firmware-build-script examples/esp32-s3-devkitc-1-n8r8/ci/build-bme280-firmware.sh \
  --out-dir examples/esp32-s3-devkitc-1-n8r8/evidence/bme280
```

The committed local smoke observes `BME280_ID=0x60` and
`BME280_TEMP_RAW=0x082DC0`. The hosted Arduino result model-verifies the
sensor-read marker against the generic DevKitC-1 catalog model, with ideal
VDD; neither result is a physical-board run.

## Model boundary

This profile exposes only the GPIO-matrix I2C0 route, BME280 model, and UART0
console used by the smoke. Although the selected commercial variant includes
8 MB OPI PSRAM, this profile does not claim PSRAM behavior. USB, Wi-Fi, BLE,
board buttons, and the USB-to-UART bridge are also outside scope. See the
committed smoke result under `evidence/bme280/`.
