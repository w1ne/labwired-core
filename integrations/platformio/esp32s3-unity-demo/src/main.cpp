// Production entry point (unused by `pio test`, required by `pio run`).
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
}

void loop() {
  Serial.println("labwired closed-loop demo");
  delay(1000);
}
