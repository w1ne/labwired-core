// Seeed Studio XIAO ESP32-C6 + BME280 I2C smoke.
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kBme280Address = 0x77;
constexpr int kSdaPin = 22; // XIAO D4
constexpr int kSclPin = 23; // XIAO D5
constexpr int kStatusLed = LED_BUILTIN; // GPIO15, active low

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
  Serial.printf("XIAO_ESP32C6_BME280_FAIL %s\n", reason);
  while (true) {
    delay(1000);
  }
}
} // namespace

void setup() {
  Serial.begin(115200); // Native USB Serial/JTAG on the XIAO Type-C connector.
  pinMode(kStatusLed, OUTPUT);
  digitalWrite(kStatusLed, HIGH); // active-low LED off until the sensor passes
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

  digitalWrite(kStatusLed, LOW); // sensor passed; active-low status LED on
  Serial.printf(
      "XIAO_ESP32C6_BME280_OK ID=0x%02X TEMP_RAW=0x%06lX "
      "I2C0=D4(GPIO22)/D5(GPIO23) LED=GPIO15\n",
      chipId, static_cast<unsigned long>(rawTemperature));
}

void loop() {
  digitalWrite(kStatusLed, HIGH);
  delay(2);
  digitalWrite(kStatusLed, LOW);
  delay(2);
}
