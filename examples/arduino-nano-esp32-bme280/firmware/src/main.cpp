// LabWired - Arduino Nano ESP32 + BME280 I2C smoke
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kBme280Address = 0x77;
// Use the board's standard I2C headers: A4=GPIO11 (SDA), A5=GPIO12 (SCL).
// Raw GPIO numbers make the sketch independent of the generic-S3 build
// variant used by the local smoke runner.
#if defined(ARDUINO_NANO_ESP32) && defined(BOARD_HAS_PIN_REMAP)
constexpr int kSdaPin = A4;
constexpr int kSclPin = A5;
#else
constexpr int kSdaPin = 11;
constexpr int kSclPin = 12;
#endif

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

void printStatus(const char *text) {
  // The smoke build routes Serial to UART0. The stock Nano profile routes it
  // to USB CDC. Emit once so hosted capture does not interleave both streams.
  Serial.println(text);
}

void fail(const char *reason) {
  char line[96];
  snprintf(line, sizeof(line), "ARDUINO_NANO_ESP32_BME280_FAIL %s", reason);
  printStatus(line);
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

  char line[160];
  snprintf(line, sizeof(line),
           "ARDUINO_NANO_ESP32_BME280_OK ID=0x%02X TEMP_RAW=0x%06lX "
           "I2C0=A4(GPIO11)/A5(GPIO12)",
           chipId, static_cast<unsigned long>(rawTemperature));
  printStatus(line);
}

void loop() { delay(1000); }
