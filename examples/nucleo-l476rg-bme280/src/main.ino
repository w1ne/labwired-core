#include <Adafruit_BME280.h>
#include <Wire.h>

Adafruit_BME280 sensor;

void setup() {
  Serial.begin(115200);
  Wire.begin();

  if (!sensor.begin(0x77, &Wire)) {
    Serial.println("BME280_MISSING");
    return;
  }

  Serial.println("BME280_OK");
  Serial.print("T=");
  Serial.print(sensor.readTemperature(), 2);
  Serial.print("C P=");
  Serial.print(sensor.readPressure() / 100.0f, 2);
  Serial.print("hPa H=");
  Serial.print(sensor.readHumidity(), 2);
  Serial.println("%RH");
}

void loop() {}
