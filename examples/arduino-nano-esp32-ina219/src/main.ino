#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kIna219Address = 0x40;

bool readRegister16(uint8_t reg, uint16_t &value) {
  Wire.beginTransmission(kIna219Address);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(kIna219Address, static_cast<uint8_t>(2)) != 2) {
    return false;
  }
  value = (static_cast<uint16_t>(Wire.read()) << 8) |
          static_cast<uint16_t>(Wire.read());
  return true;
}

bool writeRegister16(uint8_t reg, uint16_t value) {
  Wire.beginTransmission(kIna219Address);
  Wire.write(reg);
  Wire.write(static_cast<uint8_t>(value >> 8));
  Wire.write(static_cast<uint8_t>(value));
  return Wire.endTransmission() == 0;
}

void printSample() {
  uint16_t bus = 0;
  uint16_t current = 0;
  uint16_t power = 0;
  if (!readRegister16(0x02, bus) || !readRegister16(0x04, current) ||
      !readRegister16(0x03, power)) {
    Serial.println("INA219_I2C_ERROR");
    return;
  }

  const uint32_t busMillivolts = static_cast<uint32_t>(bus >> 3) * 4;
  const int32_t currentMilliamps = static_cast<int16_t>(current) / 10;
  const uint32_t powerMilliwatts = static_cast<uint32_t>(power) * 2;
  Serial.printf("INA219_VBUS_MV=%lu I_MA=%ld P_MW=%lu\n",
                static_cast<unsigned long>(busMillivolts),
                static_cast<long>(currentMilliamps),
                static_cast<unsigned long>(powerMilliwatts));
}

void setup() {
  Serial.begin(115200);
  Wire.begin(11, 12, 100000);  // A4/SDA=GPIO11, A5/SCL=GPIO12

  // Adafruit's 0.1-ohm breakout uses this 32 V / 2 A calibration point.
  if (!writeRegister16(0x00, 0x399F) || !writeRegister16(0x05, 4096)) {
    Serial.println("INA219_I2C_ERROR");
    return;
  }

  uint16_t config = 0;
  if (!readRegister16(0x00, config)) {
    Serial.println("INA219_I2C_ERROR");
    return;
  }
  Serial.printf("INA219_CONFIG=0x%04X\n", config);
  Serial.println("INA219_NANO_ESP32_READY");
}

void loop() {
  printSample();
  delay(20);
}
