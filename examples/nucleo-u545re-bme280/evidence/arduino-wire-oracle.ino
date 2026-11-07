#include <Wire.h>

void setup() {
  Serial.begin(115200);
  Wire.begin();
  Wire.beginTransmission(0x77);
  Wire.write(0xD0);
  if (Wire.endTransmission(false) != 0 || Wire.requestFrom(0x77, 1) != 1) {
    Serial.println("NUCLEO_U545RE_BME280_I2C_ERROR");
    return;
  }

  uint8_t id = Wire.read();
  if (id == 0x60) {
    Serial.println("NUCLEO_U545RE_BME280_OK=0x60");
  } else {
    Serial.println("NUCLEO_U545RE_BME280_BAD_ID");
  }
}

void loop() {}
