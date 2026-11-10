#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t MCP9808_ADDRESS = 0x18;
constexpr uint8_t MCP9808_TA = 0x05;
constexpr uint8_t MCP9808_MANUFACTURER_ID = 0x06;
constexpr uint8_t MCP9808_DEVICE_ID = 0x07;
constexpr uint16_t TEMP_MIN_X16 = 24 * 16;
constexpr uint16_t TEMP_MAX_X16 = 26 * 16;

bool read_register16(uint8_t reg, uint16_t &value, uint8_t &wire_status) {
  Wire.beginTransmission(MCP9808_ADDRESS);
  Wire.write(reg);
  // A STOP followed by a fresh START is supported by the Nano Every's
  // ArduinoCore-megaavr transfer path; the device retains its register pointer.
  wire_status = Wire.endTransmission();
  if (wire_status != 0) {
    return false;
  }

  delay(1);
  wire_status = Wire.requestFrom(static_cast<int>(MCP9808_ADDRESS), 2);
  if (wire_status != 2) {
    while (Wire.available()) {
      Wire.read();
    }
    return false;
  }

  const uint8_t msb = Wire.read();
  const uint8_t lsb = Wire.read();
  value = (static_cast<uint16_t>(msb) << 8) | lsb;
  return true;
}
} // namespace

void setup() {
  Serial.begin(115200);
  Wire.begin(); // Nano Every D20/SDA = PA2, D21/SCL = PA3

  uint16_t manufacturer_id = 0;
  uint16_t device_id = 0;
  uint16_t temperature_x16 = 0;
  uint8_t wire_status = 0;
  const uint8_t registers[] = {
      MCP9808_MANUFACTURER_ID, MCP9808_DEVICE_ID, MCP9808_TA};
  uint16_t *values[] = {&manufacturer_id, &device_id, &temperature_x16};
  for (uint8_t i = 0; i < 3; ++i) {
    if (!read_register16(registers[i], *values[i], wire_status)) {
      Serial.print("MCP9808_FAIL reg=");
      Serial.print(registers[i], HEX);
      Serial.print(" wire=");
      Serial.println(wire_status);
      return;
    }
  }

  temperature_x16 &= 0x1FFF;
  if (manufacturer_id != 0x0054 || device_id != 0x0400 ||
      temperature_x16 < TEMP_MIN_X16 || temperature_x16 > TEMP_MAX_X16) {
    Serial.print("MCP9808_FAIL id=");
    Serial.print(manufacturer_id, HEX);
    Serial.print(" dev=");
    Serial.print(device_id, HEX);
    Serial.print(" temp_x16=");
    Serial.println(temperature_x16);
    return;
  }

  Serial.print("NANO_EVERY_MCP9808_OK id=");
  Serial.print(manufacturer_id, HEX);
  Serial.print(" dev=");
  Serial.print(device_id, HEX);
  Serial.print(" temp_x16=");
  Serial.println(temperature_x16);
}

void loop() {}
