// Adafruit BME280 on STM32duino Wire: raw multi-byte reads, then the library.
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

static void tryRead(uint8_t reg, uint8_t len) {
  Wire.beginTransmission(0x76);
  Wire.write(reg);
  uint8_t e = Wire.endTransmission(false);
  uint8_t n = Wire.requestFrom((uint8_t)0x76, len);
  Serial.print("reg 0x"); Serial.print(reg, HEX); Serial.print(" len "); Serial.print(len);
  Serial.print(" end="); Serial.print(e); Serial.print(" got="); Serial.print(n); Serial.print(" :");
  while (Wire.available()) { Serial.print(" "); Serial.print(Wire.read(), HEX); }
  Serial.println();
}

void setup() {
  Serial.begin(115200);
  Wire.begin();
  tryRead(0xD0, 1);
  tryRead(0x88, 2);
  tryRead(0x88, 3);
  tryRead(0x88, 6);
  if (!bme.begin(0x76)) {
    Serial.println("BME280 not found");
  } else {
    Serial.println("BME280 ok");
    Serial.print("T=");
    Serial.print(bme.readTemperature());
    Serial.println(" C");
  }
  Serial.println("DONE");
}

void loop() {}
