// Arduino Uno sketch for the Servo twin: the Servo library on D6.
// Built with arduino-cli 1.5.1, arduino:avr 1.8.8, Servo 1.3.0:
//
//   arduino-cli compile -b arduino:avr:uno --output-dir . <this sketch>
//
// The library runs Timer1 in normal mode at clk/8 and bit-bangs the pulse
// from the TIMER1_COMPA ISR. SG90 range 500..2400 us: 150, then 30, then 90.
#include <Servo.h>

Servo s;

void setup() {
  s.attach(6, 500, 2400);
  s.write(150);
  delay(300);
  s.write(30);
  delay(300);
  s.write(90);
}

void loop() {}
