// Arduino Uno sketch for the analogWrite twin: OC2B (D3), OC2A (D11) and
// OC0B (D5). Built with arduino-cli 1.5.1, arduino:avr 1.8.8:
//
//   arduino-cli compile -b arduino:avr:uno --output-dir . <this sketch>
//
// D3 and D11 are Timer2 phase-correct PWM, D5 is Timer0 fast PWM (the
// counter millis() runs on). After 100 ms D3 goes back to a plain LOW (analogWrite(3, 0)).
#include <Arduino.h>

void setup() {
  pinMode(3, OUTPUT);
  pinMode(11, OUTPUT);
  pinMode(5, OUTPUT);
  analogWrite(3, 200);
  analogWrite(11, 50);
  analogWrite(5, 128);
  delay(100);
  analogWrite(3, 0);
}

void loop() {}
