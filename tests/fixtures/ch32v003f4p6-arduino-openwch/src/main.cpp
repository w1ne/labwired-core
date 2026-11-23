// Official openwch Arduino core (arduino_core_ch32) on CH32V003F4P6.
// delay()/millis() run on the SysTick interrupt (vector 12, HPE fast
// handler), Serial is USART1 on PD5, digitalWrite/digitalRead use GPIOC.
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  Serial.println("CH32V003_ARDUINO_BOOT");
  pinMode(PC0, OUTPUT);
  uint32_t t0 = millis();
  for (int i = 0; i < 4; i++) {
    digitalWrite(PC0, (i & 1) ? HIGH : LOW);
    delay(10);
    Serial.print("tick ");
    Serial.print(i);
    Serial.print(" pc0=");
    Serial.println(digitalRead(PC0));
  }
  uint32_t dt = millis() - t0;
  Serial.print("elapsed_ms=");
  Serial.println(dt);
  Serial.println(dt >= 40 && dt <= 42 ? "CH32V003_ARDUINO_PASS" : "CH32V003_ARDUINO_FAIL");
  Serial.println("CH32V003_ARDUINO_DONE");
}

void loop() {}
