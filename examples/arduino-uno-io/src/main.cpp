#include <Arduino.h>

// Arduino Uno R3 I/O proof for the LabWired twin: one pin on each AVR port.
//   D7 (PD7)  output - toggles every loop
//   D2 (PD2)  input with pull-up - a button to GND
//   A0 (PC0)  analog - a potentiometer wiper
//   D13 (PB5) LED_BUILTIN - mirrors the button
// Prints "D2=<level> A0=<counts>" each loop.
const int OUT_PIN = 7;
const int BUTTON_PIN = 2;

void setup() {
  pinMode(OUT_PIN, OUTPUT);
  pinMode(BUTTON_PIN, INPUT_PULLUP);
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(9600);
  Serial.println("uno-io");
}

void loop() {
  static bool out = false;
  out = !out;
  digitalWrite(OUT_PIN, out ? HIGH : LOW);
  int pressed = digitalRead(BUTTON_PIN) == LOW;
  digitalWrite(LED_BUILTIN, pressed ? HIGH : LOW);
  int a0 = analogRead(A0);
  Serial.print("D2=");
  Serial.print(pressed ? 0 : 1);
  Serial.print(" A0=");
  Serial.println(a0);
  delay(2);
}
