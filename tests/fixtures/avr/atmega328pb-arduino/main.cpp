// Stock Arduino AVR core (MiniCore variant atmega328pb) on the ATmega328PB.
// LABWIRED_NO_BLINK builds the negative control: same banner and loop timing,
// no digitalWrite, so PB5 must stay flat.
#include <Arduino.h>

static uint32_t blinks = 0;

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(9600);
  Serial.println(F("ATMEGA328PB_ARDUINO_BOOT"));
}

void loop() {
#ifndef LABWIRED_NO_BLINK
  digitalWrite(LED_BUILTIN, HIGH);
  delay(50);
  digitalWrite(LED_BUILTIN, LOW);
  delay(50);
#else
  delay(100);
#endif
  blinks++;
  if (blinks == 3) {
    Serial.println(F("ATMEGA328PB_ARDUINO_DONE"));
  }
}
