#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t BME280_ADDRESS = 0x77;

bool read_register(uint8_t reg, uint8_t *data, size_t length) {
  Wire1.beginTransmission(BME280_ADDRESS);
  Wire1.write(reg);
  if (Wire1.endTransmission(false) != 0) {
    return false;
  }

  const size_t received = Wire1.requestFrom(
      static_cast<uint8_t>(BME280_ADDRESS), static_cast<uint8_t>(length));
  if (received != length) {
    while (Wire1.available()) {
      Wire1.read();
    }
    return false;
  }

  for (size_t i = 0; i < length; ++i) {
    data[i] = static_cast<uint8_t>(Wire1.read());
  }
  return true;
}
}  // namespace

void setup() {
  Serial.begin(115200);
  Wire1.begin();  // Arduino UNO R4 WiFi Qwiic: SDA=P401, SCL=P400, IIC0.

  uint8_t chip_id = 0;
  uint8_t status = 0xFF;
  uint8_t raw_temperature[3] = {};
  if (!read_register(0xD0, &chip_id, 1) || chip_id != 0x60) {
    Serial.println("UNO_R4_WIFI_QWIIC_BME280_FAIL chip_id");
    return;
  }
  if (!read_register(0xF3, &status, 1) || status != 0x00) {
    Serial.println("UNO_R4_WIFI_QWIIC_BME280_FAIL status");
    return;
  }
  if (!read_register(0xFA, raw_temperature, sizeof(raw_temperature)) ||
      (raw_temperature[0] == 0 && raw_temperature[1] == 0 &&
       raw_temperature[2] == 0)) {
    Serial.println("UNO_R4_WIFI_QWIIC_BME280_FAIL temperature");
    return;
  }

  Serial.print("UNO_R4_WIFI_QWIIC_BME280_OK id=0x");
  Serial.print(chip_id, HEX);
  Serial.print(" status=0x");
  Serial.print(status, HEX);
  Serial.print(" temp_raw=0x");
  for (uint8_t byte : raw_temperature) {
    if (byte < 0x10) {
      Serial.print('0');
    }
    Serial.print(byte, HEX);
  }
  Serial.println();
}

void loop() {}
