#include <Arduino.h>

// Golden survival sketch for LabWired ATmega328P twin (sim-smoke bar).
// Short delay so max_cycles budgets stay small.
void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(9600);
  Serial.println("nano-ok");
}

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(1);
  digitalWrite(LED_BUILTIN, LOW);
  delay(1);
  Serial.print(".");
}
