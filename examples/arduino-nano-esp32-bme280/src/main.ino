#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kBme280Address = 0x77;

bool readRegisters(uint8_t first, uint8_t *bytes, size_t count) {
  Wire.beginTransmission(kBme280Address);
  Wire.write(first);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(kBme280Address, static_cast<uint8_t>(count)) != count) {
    return false;
  }
  for (size_t i = 0; i < count; ++i) bytes[i] = Wire.read();
  return true;
}

void printHexByte(uint8_t value) {
  constexpr char kHex[] = "0123456789ABCDEF";
  Serial.print(kHex[value >> 4]);
  Serial.print(kHex[value & 0x0F]);
}

void printHex20(uint32_t value) {
  printHexByte(static_cast<uint8_t>(value >> 16));
  printHexByte(static_cast<uint8_t>(value >> 8));
  printHexByte(static_cast<uint8_t>(value));
}

void setup() {
  Serial.begin(115200);
  Wire.begin(11, 12, 100000);  // A4/SDA=GPIO11, A5/SCL=GPIO12

  uint8_t id = 0;
  if (!readRegisters(0xD0, &id, 1)) {
    Serial.println("BME280_I2C_ERROR");
    return;
  }
  Serial.print("BME280_ID=0x");
  printHexByte(id);
  Serial.println();
  if (id != 0x60) {
    Serial.println("BME280_ID_ERROR");
    return;
  }

  uint8_t temperature[3] = {};
  if (!readRegisters(0xFA, temperature, sizeof(temperature))) {
    Serial.println("BME280_I2C_ERROR");
    return;
  }
  const uint32_t raw = (static_cast<uint32_t>(temperature[0]) << 12) |
                       (static_cast<uint32_t>(temperature[1]) << 4) |
                       (temperature[2] >> 4);
  Serial.print("BME280_TEMP_RAW=0x");
  printHex20(raw);
  Serial.println();
}

void loop() {}
