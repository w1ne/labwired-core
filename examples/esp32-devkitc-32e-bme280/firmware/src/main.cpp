// ESP32-DevKitC-32E + BME280 I2C smoke.
// GPIO23 toggles only after firmware has read the BME280 identity and data.

#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kBme280Address = 0x77;
constexpr int kSdaPin = 21;
constexpr int kSclPin = 22;
constexpr int kProofPin = 23;
bool bme280ReadPassed = false;

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
    if (Wire.available() <= 0) return false;
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
  uint8_t rawTemperature[3] = {};
  if (!readRegisters(0xD0, &chipId, 1) || chipId != 0x60 ||
      !readRegisters(0xFA, rawTemperature, sizeof(rawTemperature))) {
    return;
  }

  const uint32_t raw = (static_cast<uint32_t>(rawTemperature[0]) << 12) |
                       (static_cast<uint32_t>(rawTemperature[1]) << 4) |
                       (rawTemperature[2] >> 4);
  bme280ReadPassed = raw == 0x082DC0;
}

void loop() {
  if (!bme280ReadPassed) return;
  digitalWrite(kProofPin, !digitalRead(kProofPin));
  delay(25);
}
