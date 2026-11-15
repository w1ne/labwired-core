// Arduino Uno sketch: PINx, PCINT and INTn on pins that analogWrite drives.
// Built with PlatformIO platform atmelavr 5.3.0 (framework-arduino-avr 5.4.0),
// board = uno:
//
//   pio run   (src/main.cpp = this sketch)
//
// D6 (OC0A, Timer0 fast PWM), D9 (OC1A) and D3 (OC2B, both phase-correct)
// are polled through PIND/PINB for 20 ms each; then PCINT22 on D6 and INT1
// (rising) on D3 count the waveform edges over a 20 ms delay().
#include <Arduino.h>

volatile uint16_t npc = 0, nint1 = 0;
ISR(PCINT2_vect) { npc++; }
static void nint1_isr() { nint1++; }

// Poll a PIN register for `us` microseconds: edges and high-sample share.
static void probe(const char *name, volatile uint8_t *pinreg, uint8_t bit) {
  uint16_t edges = 0; uint32_t hi = 0, n = 0;
  uint8_t last = *pinreg & bit; uint32_t t0 = micros();
  while (micros() - t0 < 20000) {
    uint8_t v = *pinreg & bit;
    if (v != last) { edges++; last = v; }
    if (v) hi++;
    n++;
  }
  Serial.print(name); Serial.print(" edges="); Serial.print(edges);
  Serial.print(" high%="); Serial.println(n ? (hi * 100 / n) : 0);
}

void setup() {
  Serial.begin(115200);
  Serial.println("R369 start");
  analogWrite(6, 64);   // Timer0 fast PWM, ~976 Hz, 25%
  analogWrite(9, 192);  // Timer1 phase-correct, ~490 Hz, 75%
  analogWrite(3, 128);  // Timer2 phase-correct, ~490 Hz, 50%
  probe("D6", &PIND, _BV(6));
  probe("D9", &PINB, _BV(1));
  probe("D3", &PIND, _BV(3));
  // Pin-change interrupt on D6 (PCINT22) driven by the PWM pin itself.
  PCMSK2 = _BV(PCINT22); PCIFR = _BV(PCIF2); PCICR = _BV(PCIE2);
  // INT1 on D3 (OC2B): AVR external interrupts fire on output pins too.
  attachInterrupt(digitalPinToInterrupt(3), nint1_isr, RISING);
  delay(20);
  noInterrupts(); uint16_t a = npc, b = nint1; interrupts();
  PCICR = 0; detachInterrupt(digitalPinToInterrupt(3));
  Serial.print("PCINT22 n="); Serial.println(a);
  Serial.print("INT1 n="); Serial.println(b);
  Serial.println("R369 DONE");
}
void loop() {}
