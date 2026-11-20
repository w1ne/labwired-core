// Adafruit ESP32 Feather V2 (ESP32-D0WD-V3) red user LED on GPIO13.
#include <Arduino.h>

namespace {
constexpr uintptr_t kGpioBase = 0x3ff44000;
constexpr uint32_t kLedMask = 1u << 13;
} // namespace

// Local CLI fast-boot fixture. This exercises the classic ESP32 GPIO model
// without depending on Arduino/IDF startup or the board's unmodelled power rail.
extern "C" __attribute__((used, noinline)) void labwired_entry() {
  auto *enableW1ts = reinterpret_cast<volatile uint32_t *>(kGpioBase + 0x24);
  auto *outW1tc = reinterpret_cast<volatile uint32_t *>(kGpioBase + 0x0c);
  auto *outW1ts = reinterpret_cast<volatile uint32_t *>(kGpioBase + 0x08);
  *enableW1ts = kLedMask;
  *outW1ts = kLedMask;
  *outW1tc = kLedMask;
  *outW1ts = kLedMask;
  for (;;) {
    asm volatile("nop");
  }
}

// Copy-paste Arduino sketch, model-verified against the exact catalog board.
void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  digitalWrite(LED_BUILTIN, HIGH);
}

void loop() { delay(100); }
