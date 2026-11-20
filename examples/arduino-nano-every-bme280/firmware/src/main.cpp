#include <Arduino.h>
#include <Wire.h>
#include <Adafruit_BME280.h>

// Copyable ArduinoCore-megaavr example for the Nano Every with an Adafruit
// BME280 breakout on A4 (SDA, PA2) and A5 (SCL, PA3). Wire is TWI0; Serial is
// USART3 behind the board's SAMD11 USB bridge.
Adafruit_BME280 bme;

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  if (!bme.begin(0x77)) {
    Serial.println("BME280 not found at 0x77");
    return;
  }
  Serial.print("NANO_EVERY_BME280 ID=0x");
  Serial.println(bme.sensorID(), HEX);
  Serial.print("T=");
  Serial.print(bme.readTemperature(), 2);
  Serial.println(" C");
  Serial.print("P=");
  Serial.print(bme.readPressure() / 100.0F, 2);
  Serial.println(" hPa");
  Serial.print("RH=");
  Serial.print(bme.readHumidity(), 2);
  Serial.println(" %");
  digitalWrite(LED_BUILTIN, HIGH);
}

void loop() {}
