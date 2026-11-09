#include <Arduino.h>
#include <stdint.h>

void setup() {
  Serial.begin(115200);
  Serial.println("ESP32_DEVKITC32E_DAC1_BEGIN");
  dacWrite(25, 128);  // GPIO25 / DAC1, direct 8-bit output.
  delay(2);
  Serial.println("ESP32_DEVKITC32E_DAC1_CODE=128");
}

void loop() { delay(1000); }
