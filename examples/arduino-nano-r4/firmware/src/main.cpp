#include <Arduino.h>
// Arduino Nano R4 smoke: D0/D1 header UART (Serial1 = SCI2, P302 TX) and
// LED_BUILTIN (P204). Native USB Serial is not used.
void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial1.begin(115200);
  Serial1.println("NANO_R4_BOOT");
}
static uint32_t n;
void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(5);
  digitalWrite(LED_BUILTIN, LOW);
  delay(5);
  if (++n == 2) Serial1.println("NANO_R4_OK blink=2 LED=P204 UART=SCI2");
}
