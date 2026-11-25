// Official openwch Arduino core (arduino_core_ch32) on CH32V003F4P6.
// Wire reads the BME280 chip-id register (0xD0) over I2C1 on the default
// SDA=PC1 / SCL=PC2 pads. The core enables the I2C clock, programs CKCFGR,
// runs the EV/ER interrupt path in twi.c, and polls STAR1/STAR2 through the
// vendor I2C_GetFlagStatus(). The chip id (0x60) proves both an address phase
// and a data phase crossed the model.
#include <Arduino.h>
#include <Wire.h>

static const uint8_t kBme280 = 0x76;

void setup() {
  Serial.begin(115200);
  Serial.println("CH32V003_I2C_BOOT");

  Wire.begin();

  Wire.beginTransmission(kBme280);
  Wire.write(0xD0);
  uint8_t err = Wire.endTransmission();

  uint8_t count = Wire.requestFrom(kBme280, (uint8_t)1);
  uint8_t id = (count == 1) ? (uint8_t)Wire.read() : 0;

  Serial.print("err=");
  Serial.println(err);
  Serial.print("count=");
  Serial.println(count);
  Serial.print("id=0x");
  Serial.println(id, HEX);
  Serial.println((err == 0 && count == 1 && id == 0x60) ? "CH32V003_I2C_PASS"
                                                        : "CH32V003_I2C_FAIL");
  Serial.println("CH32V003_I2C_DONE");
}

void loop() {}
