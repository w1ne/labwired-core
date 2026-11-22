#include <Arduino.h>
#include <Wire.h>
#include <Adafruit_BME280.h>

// Copyable ArduinoCore-mbed example for the Nano 33 BLE Rev2 with an Adafruit
// BME280 breakout on A4 (SDA, P0.31) and A5 (SCL, P0.02). Wire is the
// nRF52840's TWI0; Serial is the native USB port as a CDC serial device.
Adafruit_BME280 bme;

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  // Native USB: wait until a terminal opens the port (DTR), or the first
  // lines are lost.
  while (!Serial) {
    delay(1);
  }
  if (!bme.begin(0x77)) {
    Serial.println("BME280 not found at 0x77");
    return;
  }
  Serial.print("NANO_33_BLE_BME280 ID=0x");
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
