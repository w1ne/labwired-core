// Arduino Mega 2560 sketch for the analogWrite twin: every Timer0/1/2
// output-compare pin. Built with PlatformIO atmelavr 5.3.0, framework-arduino-avr
// 5.4.0 (ArduinoCore-avr), board megaatmega2560:
//
//   pio run   (platformio.ini: platform = atmelavr, board = megaatmega2560,
//              framework = arduino)
//
// Mega pin -> OC unit (DS2549 section 13.3; Arduino pins_arduino.h "mega"):
//   D13 = PB7 = OC0A, D4 = PG5 = OC0B  (Timer0 fast PWM, 976 Hz)
//   D11 = PB5 = OC1A, D12 = PB6 = OC1B (Timer1 phase-correct 8-bit, 490 Hz)
//   D10 = PB4 = OC2A, D9 = PH6 = OC2B  (Timer2 phase-correct, 490 Hz)
// After 100 ms D9 goes back to a plain LOW (analogWrite(9, 0)).
#include <Arduino.h>

static const uint8_t pins[] = {13, 4, 11, 12, 10, 9};
static const uint8_t duty[] = {128, 64, 200, 50, 100, 180};

void setup() {
  for (uint8_t i = 0; i < sizeof(pins); i++) {
    pinMode(pins[i], OUTPUT);
    analogWrite(pins[i], duty[i]);
  }
  delay(100);
  analogWrite(9, 0);
}

void loop() {}
