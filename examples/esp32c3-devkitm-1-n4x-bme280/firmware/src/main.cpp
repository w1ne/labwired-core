// ESP32-C3-DevKitM-1-N4X + BME280 I2C smoke.
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kBme280Address = 0x77;
constexpr int kSdaPin = 4;
constexpr int kSclPin = 5;

bool readRegisters(uint8_t firstRegister, uint8_t *destination, size_t length) {
  Wire.beginTransmission(kBme280Address);
  if (Wire.write(firstRegister) != 1 || Wire.endTransmission(false) != 0) {
    return false;
  }

  if (Wire.requestFrom(static_cast<uint16_t>(kBme280Address), length, true) !=
      length) {
    return false;
  }
  for (size_t i = 0; i < length; ++i) {
    if (Wire.available() <= 0) {
      return false;
    }
    destination[i] = static_cast<uint8_t>(Wire.read());
  }
  return true;
}

void fail(const char *reason) {
  Serial.printf("ESP32C3_DEVKITM1_N4X_BME280_FAIL %s\n", reason);
  while (true) {
    delay(1000);
  }
}
} // namespace

void setup() {
  Serial.begin(115200);
  Wire.begin(kSdaPin, kSclPin, 100000);

  uint8_t chipId = 0;
  if (!readRegisters(0xD0, &chipId, 1)) {
    fail("I2C_READ");
  }
  if (chipId != 0x60) {
    fail("CHIP_ID");
  }

  uint8_t temperature[3] = {};
  if (!readRegisters(0xFA, temperature, sizeof(temperature))) {
    fail("TEMP_READ");
  }
  const uint32_t rawTemperature =
      (static_cast<uint32_t>(temperature[0]) << 12) |
      (static_cast<uint32_t>(temperature[1]) << 4) |
      (static_cast<uint32_t>(temperature[2]) >> 4);

  Serial.printf(
      "ESP32C3_DEVKITM1_N4X_BME280_OK ID=0x%02X TEMP_RAW=0x%06lX "
      "I2C0=GPIO4/5\n",
      chipId, static_cast<unsigned long>(rawTemperature));
}

void loop() { delay(1000); }
