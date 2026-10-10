#include <Wire.h>
#include <Adafruit_BME280.h>
Adafruit_BME280 bme;
void setup() {
  Serial.begin(115200);
  Serial.println("boot"); if (!bme.begin(0x76)) { Serial.println("BME280 not found"); while (1) {} }
  Serial.println("BME280 ok");
}
void loop() {
  Serial.print("T=");
  Serial.print(bme.readTemperature());
  Serial.println(" C");
  delay(500);
}
