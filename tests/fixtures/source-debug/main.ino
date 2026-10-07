// Source-stepping fixture: a loop() line that calls a function, a function
// that calls another, and a delay to step over.
#include <Arduino.h>

static const int LED = 2;
volatile int ticks = 0;

int twice(int v) {
  int r = v * 2;
  ticks = ticks + r;
  return r;
}

int bump(int v) {
  int t = twice(v);
  ticks = ticks + 1;
  return t + 1;
}

void setup() {
  Serial.begin(115200);
  pinMode(LED, OUTPUT);
}

void loop() {
  digitalWrite(LED, HIGH);
  int n = bump(ticks);
  Serial.println(n);
  digitalWrite(LED, LOW);
  delay(100);
}
