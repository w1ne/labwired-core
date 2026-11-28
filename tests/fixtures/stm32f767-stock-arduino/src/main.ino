#include <Arduino.h>
void setup() { Serial.begin(115200); pinMode(LED_BUILTIN, OUTPUT); }
void loop() { Serial.println("F767_OK"); digitalWrite(LED_BUILTIN, HIGH); delay(20); digitalWrite(LED_BUILTIN, LOW); delay(20); }
