# ESP32-C6-DevKitC-1-N8 + BME280

## Demand position

ESP32 is the survey's **#1 MCU family** (38% of respondents; ESP32 boards are
42% of kits). This adds board-level coverage to the existing ESP32-C6 model;
it is maintenance and board depth, not new silicon. The exact N8 board has
8 MB flash, and this profile connects only UART0 and I2C0. See the [live
availability receipt](evidence/bme280/vendor-status.md).

## Wiring

Connect an Adafruit BME280 STEMMA QT breakout at address `0x77`:

| BME280 breakout | ESP32-C6-DevKitC-1-N8 |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | GPIO6 (J1 pin 5) |
| SCL | GPIO7 (J1 pin 6) |

The firmware routes I2C0 through the C6 GPIO matrix on GPIO6/GPIO7, reads the
BME280 chip ID (`0x60`) and a raw temperature register sample, then prints
both results on UART0. The physical board routes UART0 through a USB-to-UART
bridge; simulation captures UART0 directly and does not model that bridge.

## Copy-paste Arduino example

Install Adafruit's `Adafruit BME280 Library` and `Adafruit Unified Sensor`
libraries, connect the breakout as shown above, and upload:

```cpp
#include <Adafruit_BME280.h>
#include <Arduino.h>
#include <Wire.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin(6, 7, 100000);  // SDA=GPIO6, SCL=GPIO7
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    return;
  }
  Serial.printf("BME280_ID=0x%02X\n", bme.sensorID());
  Serial.printf("Temperature C: %.2f\n", bme.readTemperature());
}

void loop() {}
```

## Reproduce the local simulator smoke

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c6-devkitc-1-n8-bme280 \
  --crate firmware-esp32c6-devkitc-bme280-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c6-devkitc-1-n8-bme280/bme280-smoke.yaml \
  --system configs/systems/esp32c6-devkitc-1-n8-bme280.yaml \
  --firmware-build-script examples/esp32c6-devkitc-1-n8-bme280/ci/build-bme280-firmware.sh \
  --out-dir examples/esp32c6-devkitc-1-n8-bme280/evidence/bme280
```

The profile does not claim Wi-Fi, Bluetooth, 802.15.4, USB behavior, board
buttons, RGB LED behavior, LP-core behavior, or physical-board validation.
The hosted catalog currently lists the C6 DevKitC as `design_only`; this
repository has a passing local CLI smoke, not a hosted `labwired_verify`
verdict.
