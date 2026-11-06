#include <avr/io.h>

// Arduino Nano Every D13 / LED_BUILTIN is PE2. This example uses the
// ATmega4809's VPORT-E registers directly, avoiding timer-dependent delay().
extern "C" void setup() {
  VPORTE.DIR |= PIN2_bm;
  VPORTE.OUT &= static_cast<uint8_t>(~PIN2_bm);
}

extern "C" void loop() {
  VPORTE.OUT ^= PIN2_bm;
  for (uint16_t spin = 0; spin < 200; ++spin) {
    asm volatile("nop");
  }
}
