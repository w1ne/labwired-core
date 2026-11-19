#include <Arduino.h>

// Copyable ArduinoCore-megaavr example for the Nano Every. D13 / LED_BUILTIN
// is PE2, and Serial is USART3 behind the board's SAMD11 USB bridge.
void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  Serial.println("NANO_EVERY_OK");
}

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(1);
  digitalWrite(LED_BUILTIN, LOW);
  delay(1);
}
