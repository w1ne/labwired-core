# Arduino Nano ESP32 + BME280

## Demand position

ESP32 is demand rank **1** (38% of reported MCU use); Arduino boards appeared
in 37% of respondents' main-project kits. This is board and sensor coverage for
the already-supported ESP32-S3, not a new silicon family. Arduino currently
lists the Nano ESP32 as SKU ABX00092. Live status and the source pin map are in
[`vendor-status.md`](vendor-status.md).

## Wiring

Connect an Adafruit BME280 STEMMA QT breakout:

| BME280 | Arduino Nano ESP32 |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | A4 / GPIO11 |
| SCL | A5 / GPIO12 |

The Adafruit breakout defaults to address `0x77`; leave SDO unconnected on the
physical breakout. These are the GPIO numbers in Arduino's official Nano ESP32
pin map. This example names them explicitly so the board's physical A4/A5
route is clear.

## Copy-paste Arduino sketch

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t SDA_PIN = 11;
constexpr uint8_t SCL_PIN = 12;
constexpr uint8_t BME280_ADDR = 0x77;

void setup() {
  Serial.begin(115200);
  Wire.begin(SDA_PIN, SCL_PIN, 100000);
  Wire.beginTransmission(BME280_ADDR);
  Wire.write(0xD0);  // BME280 CHIP_ID register.
  if (Wire.endTransmission(false) != 0 || Wire.requestFrom(BME280_ADDR, 1) != 1) {
    Serial.println("NANO_ESP32_BME280_I2C_ERROR");
    return;
  }
  const uint8_t id = Wire.read();
  Serial.printf("NANO_ESP32_BME280_ID=0x%02X\n", id);
}

void loop() {}
```

## Smoke evidence

Run the local system-backed onboarding smoke:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id arduino-nano-esp32-bme280 \
  --script examples/arduino-nano-esp32-bme280/bme280-smoke.yaml \
  --system configs/systems/arduino-nano-esp32-bme280.yaml \
  --firmware-build-script examples/arduino-nano-esp32-bme280/ci/build-bme280-firmware.sh \
  --out-dir examples/arduino-nano-esp32-bme280/evidence
```

The committed local fixture uses the S3 I2C0 command-list model on GPIO11/12.
It asserts BME280 ID `0x60` and the deterministic raw temperature sample
`0x082DC0`. Its fast-boot entry bypasses Arduino startup; it tests the sensor
transaction and the board's configured pin route. It does not claim live
environmental measurements or physical-board behavior.

The copy-paste Arduino sketch was also hosted-verified against an explicit
Nano ESP32 + BME280 diagram. The serial oracle saw
`NANO_ESP32_BME280_ID=0x60`; the exact request and response are in
[`evidence/hosted-verify.json`](evidence/hosted-verify.json). The hosted board
catalog's default A4/A5 route says GPIO8/GPIO9, which conflicts with Arduino's
official GPIO11/GPIO12 map. The hosted verification therefore wires GPIO11 and
GPIO12 explicitly and the sketch passes those pins to `Wire.begin`. The
verification also reports an ideal-rail approximation: power-on and brown-out
behavior are not simulated. A separate diagram validation warned that its
I2C pull-up resistors are not drawn; that warning is recorded in the evidence.
The hosted BME280 model defaults to `0x76`, so its diagram ties SDO to 3V3 to
represent the physical breakout's default `0x77` address selection.
Add an explicit board power model if power sequencing or brown-out behavior
becomes part of this profile's acceptance scope.

## Scope and limits

The profile claims I2C0 to the BME280 and UART0 output. The ESP32-S3 chip YAML
still declares the `SYSTEM` address window as an explicit `system_stub`; this
smoke does not access it. Wi-Fi/Bluetooth, USB-OTG, PSRAM, board power, the
RGB LED, and boot behavior are outside this profile. The connected hosted
catalog's default A4/A5 description currently reports GPIO8/GPIO9, which
conflicts with Arduino's official pinout. The explicit GPIO11/GPIO12 wiring in
the hosted evidence lets the sensor transaction verify without relying on
those defaults. The local system runner and hosted result both use the
vendor-published pins.
