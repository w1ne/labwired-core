#include <Adafruit_SHT4x.h>
#include <Arduino.h>
#include <Wire.h>

Adafruit_SHT4x sht4;

void setup() {
  Serial.begin(115200);
  Wire1.begin(41, 42, 100000);  // I2C1 SDA=GPIO41, SCL=GPIO42
  if (!sht4.begin(&Wire1)) {
    Serial.println("SHT40 not found on I2C1");
    return;
  }
  sht4.setPrecision(SHT4X_HIGH_PRECISION);

  sensors_event_t humidity, temperature;
  if (sht4.getEvent(&humidity, &temperature)) {
    Serial.printf("Temperature: %.2f C, RH: %.2f %%\n",
                  temperature.temperature, humidity.relative_humidity);
  }
}

void loop() {}
