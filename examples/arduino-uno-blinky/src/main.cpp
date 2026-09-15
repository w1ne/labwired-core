#include <Arduino.h>

// Golden survival sketch for the LabWired Arduino Uno R3 (ATmega328P) twin.
// Short delay so max_cycles budgets stay small.
void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(9600);
  Serial.println("uno-ok");
}

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(1);
  digitalWrite(LED_BUILTIN, LOW);
  delay(1);
  Serial.print(".");
}
