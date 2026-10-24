// Arduino Mega 2560 I/O smoke: the pins, ADC channels, timer and USART a
// typical sketch uses on the Mega, as exercised by
// crates/core/tests/avr_mega2560_io.rs.
//
// Every 100 ms (millis(), Timer0) one of 12 LEDs is lit, in the order
// D3 D4 D5 A2 A1 A0 D13 D12 D8 D7 D6 D2 (ports E, G, F, B, H), and one line goes
// out on Serial (USART0, 9600):
//   k=<index> a8=<analogRead(A8)> a9=<analogRead(A9)> a0=<analogRead(A0)> ms=<millis()> f=<fn>
// A8/A9 are ADC8/ADC9 (ADCSRB.MUX5); A0 is ADC0 (MUX5 cleared again). `f`
// comes through a function pointer (an EICALL on the 3-byte-PC part).
#include <Arduino.h>

static const uint8_t PINS[12] = {3, 4, 5, A2, A1, A0, 13, 12, 8, 7, 6, 2};

static int twice(int v) { return 2 * v; }
static int thrice(int v) { return 3 * v; }
int (*volatile pick)(int) = twice;

static unsigned long last = 0;
static uint8_t k = 0;

void setup() {
  Serial.begin(9600);
  for (uint8_t i = 0; i < 12; i++) pinMode(PINS[i], OUTPUT);
  Serial.println(F("MEGA READY"));
}

void loop() {
  if (millis() - last >= 100) {
    last += 100;
    for (uint8_t i = 0; i < 12; i++) digitalWrite(PINS[i], i == k ? HIGH : LOW);
    int a8 = analogRead(A8);
    int a9 = analogRead(A9);
    int a0 = analogRead(A0);
    pick = (k & 1) ? thrice : twice;
    Serial.print(F("k="));
    Serial.print(k);
    Serial.print(F(" a8="));
    Serial.print(a8);
    Serial.print(F(" a9="));
    Serial.print(a9);
    Serial.print(F(" a0="));
    Serial.print(a0);
    Serial.print(F(" ms="));
    Serial.print(millis());
    Serial.print(F(" f="));
    Serial.println(pick(k + 1));
    k = (k + 1) % 12;
  }
}
