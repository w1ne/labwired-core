#include <Arduino.h>

// Golden survival sketch for LabWired ATmega328P twin (sim-smoke bar).
// Short delay so max_cycles budgets stay small.
static bool announced = false;

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(9600);
}

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(1);
  digitalWrite(LED_BUILTIN, LOW);
  delay(1);
  if (!announced) {
    Serial.println("nano-ok");
    announced = true;
  } else {
    Serial.print(".");
  }
}
