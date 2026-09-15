// LabWired Arduino matrix L7 — free-running time base (timers behind micros).
//
// Proves: the board's Arduino time base advances over a short wait
// (SysTick / RTC1 / TIMG / RP2040 timer / AVR Timer0 — whatever drives micros).
// Stronger than L1: requires a measured micros() delta, not only delay() return.

#include <Arduino.h>

static void logBegin() {
  Serial.begin(115200);
#if defined(ARDUINO_USB_CDC_ON_BOOT) && (ARDUINO_USB_CDC_ON_BOOT)
  Serial.setTxTimeoutMs(0);
  Serial0.begin(115200);
#endif
  delay(1);
}

static void logLine(const char *s) {
  Serial.println(s);
#if defined(ARDUINO_USB_CDC_ON_BOOT) && (ARDUINO_USB_CDC_ON_BOOT)
  Serial0.println(s);
#endif
}

void setup() {
  logBegin();
  logLine("LW_L7_BOOT");

  // Three samples: require strict advance across a real delay and monotonicity
  // after a short busy spin (covers SysTick/RTC/TIMG free-running path).
  unsigned long t0 = micros();
  delay(2);
  unsigned long t1 = micros();
  volatile uint32_t spin = 0;
  for (uint32_t i = 0; i < 2000u; i++) {
    spin += i;
  }
  (void)spin;
  unsigned long t2 = micros();
  delay(1);
  unsigned long t3 = micros();

  bool advanced = (t1 > t0) && ((t1 - t0) >= 1000UL) && (t3 > t1);
  bool mono = (t2 >= t1) && (t3 >= t2);

  if (advanced && mono) {
    logLine("LW_L7_OK");
    return;
  }
  char buf[72];
  snprintf(buf, sizeof(buf), "LW_L7_FAIL t0=%lu t1=%lu t2=%lu t3=%lu", t0, t1, t2, t3);
  logLine(buf);
}

void loop() {}
