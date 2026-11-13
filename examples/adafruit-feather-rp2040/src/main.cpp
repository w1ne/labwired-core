// Adafruit Feather RP2040 STEMMA QT BME280 chip-ID smoke.
#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kBme280Address = 0x77;
constexpr uint8_t kChipIdRegister = 0xD0;
constexpr uint8_t kBme280ChipId = 0x60;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(); // Feather's STEMMA QT connector is I2C1 on GP2/GP3.
  Wire.setClock(100000);

  Wire.beginTransmission(kBme280Address);
  Wire.write(kChipIdRegister);
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(kBme280Address, static_cast<uint8_t>(1)) != 1) {
    Serial.println("BME280 not found at 0x77");
    return;
  }

  const uint8_t chipId = Wire.read();
  if (chipId == kBme280ChipId) {
    pinMode(13, OUTPUT); // D13 is a header output; add an LED externally if desired.
    digitalWrite(13, HIGH);
    Serial.println("FEATHER_RP2040_BME280_OK I2C1=GP2/GP3 ID=0x60");
  } else {
    Serial.println("Unexpected BME280 chip ID");
  }
}

void loop() { delay(100); }
