// Seeed Studio XIAO ESP32-C3 + BME280 I2C smoke.
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kBme280Address = 0x77;
constexpr int kSdaPin = 6;      // XIAO D4
constexpr int kSclPin = 7;      // XIAO D5
constexpr int kProofPin = 10;   // XIAO D10; use an external LED + resistor
bool sensorReadPassed = false;

bool readRegisters(uint8_t firstRegister, uint8_t *destination,
                   size_t length) {
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

} // namespace

void setup() {
  pinMode(kProofPin, OUTPUT);
  digitalWrite(kProofPin, LOW);
  Wire.begin(kSdaPin, kSclPin, 100000);

  uint8_t chipId = 0;
  if (!readRegisters(0xD0, &chipId, 1)) {
    return;
  }
  if (chipId != 0x60) {
    return;
  }

  uint8_t temperature[3] = {};
  if (!readRegisters(0xFA, temperature, sizeof(temperature))) {
    return;
  }
  const uint32_t rawTemperature =
      (static_cast<uint32_t>(temperature[0]) << 12) |
      (static_cast<uint32_t>(temperature[1]) << 4) |
      (static_cast<uint32_t>(temperature[2]) >> 4);
  if (rawTemperature != 0x082DC0) {
    return;
  }
  sensorReadPassed = true;
}

void loop() {
  if (!sensorReadPassed) {
    return;
  }
  digitalWrite(kProofPin, HIGH);
  delay(2);
  digitalWrite(kProofPin, LOW);
  delay(2);
}
