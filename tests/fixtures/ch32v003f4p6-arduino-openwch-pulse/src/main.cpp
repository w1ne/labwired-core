// Official openwch Arduino core (arduino_core_ch32) on CH32V003F4P6.
// Measures one external pulse on PD3 three ways, so the simulator can show
// both the working time base and the defect in the core's micros():
//
// 1. WIDTH lines: poll GPIOD->INDR and read the QingKe V2 STK counter
//    (SysTick->CNT, which counts up from 0 to CMP and reloads) directly.
//    Elapsed microseconds = CNT * 1000 / (CMP + 1), plus the millisecond
//    tick the SysTick handler adds. This is the time base the chip has.
// 2. MICROS line: three consecutive micros() samples from the stock core
//    inside one millisecond. On this variant getCurrentMicros() computes
//    (CMP + 1 - CNT) * 1000 / (CMP + 1), which is the *remaining* time in the
//    millisecond for an up-counter, so the samples decrease.
// 3. PULSEIN line: pulseIn(PD3, HIGH) from the stock core. It measures with
//    micros(), so a real pulse on the pad cannot pass the timeout test.
#include <Arduino.h>

static uint32_t stk_micros() {
  uint32_t per_ms = (uint32_t)SysTick->CMP + 1;
  // Read the tick, the counter and the tick again so a millisecond rollover
  // between the two reads cannot mix a tick from one millisecond with a
  // counter value from the next.
  for (;;) {
    uint32_t t0 = millis();
    uint32_t cnt = (uint32_t)SysTick->CNT;
    uint32_t t1 = millis();
    if (t0 == t1) {
      return t0 * 1000u + (cnt * 1000u) / per_ms;
    }
  }
}

static inline bool pd3() { return (GPIOD->INDR & (1u << 3)) != 0; }

void setup() {
  Serial.begin(115200);
  Serial.println("CH32V003_PULSE_BOOT");
  pinMode(PD3, INPUT);

  for (int i = 0; i < 2; i++) {
    while (!pd3()) {}
    uint32_t start = stk_micros();
    while (pd3()) {}
    uint32_t stop = stk_micros();
    Serial.print("WIDTH ");
    Serial.println(stop - start);
  }

  uint32_t m0 = micros();
  uint32_t m1 = micros();
  uint32_t m2 = micros();
  Serial.print("MICROS ");
  Serial.print(m0);
  Serial.print(" ");
  Serial.print(m1);
  Serial.print(" ");
  Serial.println(m2);

  Serial.print("PULSEIN ");
  Serial.println(pulseIn(PD3, HIGH, 20000));
  Serial.println("CH32V003_PULSE_DONE");
}

void loop() {}
