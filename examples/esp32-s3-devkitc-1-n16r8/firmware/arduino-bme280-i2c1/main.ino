#include <Adafruit_BME280.h>
#include <Arduino.h>
#include <Wire.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire1.begin(41, 42, 100000);  // I2C1 SDA=GPIO41, SCL=GPIO42
  if (!bme.begin(0x77, &Wire1)) {
    Serial.println("BME280 not found on I2C1");
    return;
  }
  Serial.printf("BME280_ID=0x%02X\n", bme.sensorID());
  Serial.printf("Temperature C: %.2f\n", bme.readTemperature());
}

void loop() {}
