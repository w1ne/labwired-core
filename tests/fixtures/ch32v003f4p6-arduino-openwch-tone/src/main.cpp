// Official openwch Arduino core (arduino_core_ch32) on CH32V003F4P6.
// tone(PD4, 1000) uses the core's Tone library, which on this variant picks
// TIMER_TONE = TIM2. It sets the timer overflow to 2*frequency (2000 Hz) with
// the timer clock, attaches an update-interrupt callback that toggles PD4,
// and enables TIM2_IRQn. Each overflow toggles the pad, so the pad carries a
// 1 kHz square wave produced entirely inside the interrupt handler; the
// sketch itself never touches PD4 after tone() returns.
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  Serial.println("CH32V003_TONE_BOOT");
  tone(PD4, 1000);
  Serial.println("CH32V003_TONE_SET");
  Serial.println("CH32V003_TONE_DONE");
}

void loop() {}
