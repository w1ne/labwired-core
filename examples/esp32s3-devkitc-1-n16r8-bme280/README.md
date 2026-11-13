# ESP32-S3-DevKitC-1 N16R8 + BME280

## Demand position

ESP32 is demand rank **1** (38% of surveyed MCU use; ESP32 boards were 42% of
main-project kits). This adds a tested board-and-sensor profile over the
existing ESP32-S3 model. It is board depth, not new silicon. Live availability
and the exact current SKU are recorded in [vendor-status.md](vendor-status.md).

## Wiring

Use the Espressif ESP32-S3-DevKitC-1 v1.1 with the N16R8 module and an Adafruit
BME280 STEMMA QT breakout:

| Breakout | Board |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | GPIO8 (J1 pin 12) |
| SCL | GPIO9 (J1 pin 15) |

The board routes I2C0 through the GPIO matrix. The breakout's address is 0x77.
The copy-paste sketch below reads BME280 CHIP_ID (0xD0) and reports it over
UART0. The local fixture also reads the deterministic raw temperature
registers; it does not claim live environmental sensing.

## Copy-paste Arduino sketch

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t SDA_PIN = 8;
constexpr uint8_t SCL_PIN = 9;
constexpr uint8_t BME280_ADDR = 0x77;

bool readRegister(uint8_t reg, uint8_t &value) {
  Wire.beginTransmission(BME280_ADDR);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0 || Wire.requestFrom(BME280_ADDR, 1) != 1) {
    return false;
  }
  value = Wire.read();
  return true;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(SDA_PIN, SCL_PIN, 100000);
  uint8_t id = 0;
  if (readRegister(0xD0, id) && id == 0x60) {
    Serial.printf("BME280_ID=0x%02X\n", id);
  } else {
    Serial.println("BME280_I2C_ERROR");
  }
}

void loop() {}
```

## Local smoke

The committed fast-boot fixture runs the S3 I2C0 command-list reads directly
and asserts both the sensor ID and raw temperature sample. Its entry point
bypasses Arduino startup; the hosted Arduino oracle in the linked evidence
exercises the normal setup() path and returns model_verified for the serial
marker. That hosted run assumes an ideal MCU supply, so power-on and brown-out
behavior are not part of the result.

See [hosted verification evidence](evidence/hosted-verify.json).

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh --target-id esp32s3-devkitc-1-n16r8-bme280 --script examples/esp32s3-devkitc-1-n16r8-bme280/bme280-smoke.yaml --system configs/systems/esp32s3-devkitc-1-n16r8-bme280.yaml --firmware-build-script examples/esp32s3-devkitc-1-n16r8-bme280/ci/build-bme280-firmware.sh --out-dir examples/esp32s3-devkitc-1-n16r8-bme280/evidence
```

## Limits

The profile covers UART0 and I2C0 with this BME280 breakout. It does not claim
Wi-Fi/Bluetooth, USB-OTG, PSRAM, the RGB LED, buttons, board power behavior,
or physical sensor accuracy. The USB-to-UART bridge is abstracted by UART0.
