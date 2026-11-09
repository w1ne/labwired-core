#include <Arduino.h>
#include <stdint.h>

void setup() {
  Serial.begin(115200);
  Serial.println("ESP32_DEVKITC32E_ADC1_BEGIN");
  analogReadResolution(12);
  const int raw = analogRead(36);  // GPIO36 = ADC1_CH0 (VP).
  Serial.printf("ESP32_DEVKITC32E_ADC1_CH0=%d\n", raw);
}

void loop() {}
