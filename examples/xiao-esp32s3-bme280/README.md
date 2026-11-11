# Seeed Studio XIAO ESP32-S3 + BME280

Demand rank: **ESP32 #1 (38%)**. This example pairs the already modelled XIAO
ESP32-S3 with an external BME280 breakout and exercises the real ESP32-S3 I2C0
and GPIO21 models. It adds a practical sensor profile to the board; it does
not add a new ESP32 chip model.

## Wiring

Connect an I2C BME280 breakout to the XIAO:

| XIAO | BME280 breakout |
|---|---|
| 3V3 | VIN / 3V |
| GND | GND |
| D4 / GPIO5 / SDA | SDA / SDI |
| D5 / GPIO6 / SCL | SCL / SCK |

The modeled device uses address `0x77`. The simulation models register
responses; it does not model voltage levels, pull-up resistance, bus
capacitance, or environmental conversion accuracy.

## Copy-paste Arduino example

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kBme280Address = 0x77;
constexpr int kSdaPin = 5; // XIAO D4
constexpr int kSclPin = 6; // XIAO D5

bool readRegister(uint8_t reg, uint8_t &value) {
  Wire.beginTransmission(kBme280Address);
  if (Wire.write(reg) != 1 || Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(static_cast<uint16_t>(kBme280Address), 1, true) != 1)
    return false;
  value = static_cast<uint8_t>(Wire.read());
  return true;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(kSdaPin, kSclPin, 100000);

  uint8_t chipId = 0;
  if (readRegister(0xD0, chipId) && chipId == 0x60) {
    Serial.printf("BME280 detected: 0x%02X\n", chipId);
  } else {
    Serial.println("BME280 read failed");
  }
}

void loop() { delay(1000); }
```

## Reproduce the simulator smoke

```sh
scripts/onboarding_smoke.sh \
  --target-id xiao-esp32s3-bme280 \
  --firmware-build-script examples/xiao-esp32s3-bme280/firmware/build-firmware.sh \
  --script examples/xiao-esp32s3-bme280/bme280-smoke.yaml \
  --system configs/systems/xiao-esp32s3-bme280.yaml \
  --out-dir examples/xiao-esp32s3-bme280/evidence
```

The exact vendor availability check and evidence scope are recorded in
[vendor-status.md](vendor-status.md) and [VALIDATION.md](VALIDATION.md).
