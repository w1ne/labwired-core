// LabWired capacitive touch lab — Arduino Nano (ATmega328P).
//
// Adapted from CapacitiveSensorSketch, part of
// https://github.com/PaulStoffregen/CapacitiveSensor (MIT license):
//
//   CapitiveSense Library Demo Sketch
//   Paul Badger 2008
//   Uses a high value resistor e.g. 10M between send pin and receive pin
//   Resistor effects sensitivity, experiment with values, 50K - 50M. Larger
//   resistor values yield larger sensor values.
//   Receive pin is the sensor pin - try different amounts of foil/metal on
//   this pin
//
// Reduced to a single sensor (send D4, receive D2, matching the canvas's
// R1 1 MOhm between D4 and the touch pad on D2) and adapted to light the
// onboard LED (D13) once the reading crosses THRESHOLD.
#include <Arduino.h>
#include <CapacitiveSensor.h>

// released median 5, pressed median 3043 (simulated, R1 1 MOhm, pad 20 pF,
// finger 100 pF, AVR machine clock) -- halfway between the two.
const long THRESHOLD = 1524;

CapacitiveSensor cs_4_2 = CapacitiveSensor(4, 2); // send D4, receive D2

void setup() {
  // Library README: turn off autocalibrate for a fixed baseline.
  cs_4_2.set_CS_AutocaL_Millis(0xFFFFFFFF);
  Serial.begin(9600);
  pinMode(LED_BUILTIN, OUTPUT);
}

void loop() {
  long total = cs_4_2.capacitiveSensor(30);

  Serial.print("total=");
  Serial.println(total);

  if (total > THRESHOLD) {
    digitalWrite(LED_BUILTIN, HIGH);
  } else {
    digitalWrite(LED_BUILTIN, LOW);
  }
}
