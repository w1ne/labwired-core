#include <Wire.h>

constexpr uint8_t MCP9808_ADDRESS = 0x18;
constexpr uint16_t MCP9808_TEMP_MIN_X16 = 24 * 16;
constexpr uint16_t MCP9808_TEMP_MAX_X16 = 26 * 16;

bool read_register16(uint8_t reg, uint16_t &value) {
  Wire.beginTransmission(MCP9808_ADDRESS);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) {
    return false;
  }

  if (Wire.requestFrom(MCP9808_ADDRESS, static_cast<uint8_t>(2)) != 2) {
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

void setup() {
  Serial.begin(115200);
  Wire.begin();

  uint16_t manufacturer_id = 0;
  uint16_t device_id = 0;
  uint16_t temperature_x16 = 0;
  if (!read_register16(0x06, manufacturer_id) ||
      !read_register16(0x07, device_id) ||
      !read_register16(0x05, temperature_x16)) {
    Serial.println("MCP9808_FAIL i2c_read");
    return;
  }

  temperature_x16 &= 0x1FFF;
  if (manufacturer_id != 0x0054 || device_id != 0x0400 ||
      temperature_x16 < MCP9808_TEMP_MIN_X16 ||
      temperature_x16 > MCP9808_TEMP_MAX_X16) {
    Serial.print("MCP9808_FAIL id=");
    Serial.print(manufacturer_id, HEX);
    Serial.print(" dev=");
    Serial.print(device_id, HEX);
    Serial.print(" temp_x16=");
    Serial.println(temperature_x16);
    return;
  }

  Serial.print("MCP9808_OK id=");
  Serial.print(manufacturer_id, HEX);
  Serial.print(" dev=");
  Serial.print(device_id, HEX);
  Serial.print(" temp_x16=");
  Serial.println(temperature_x16);
}

void loop() {}
