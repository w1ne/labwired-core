// Arduino Nano ESP32 (NORA-W106 / ESP32-S3) built-in D13 LED smoke.
// The board pinout maps D13 / LED_BUILTIN to ESP32-S3 GPIO48.
#include <Arduino.h>
#include "driver/gpio.h"

namespace {
constexpr uintptr_t kGpioBase = 0x60004000;
constexpr uint32_t kGpio48BankBit = 1u << 16;
} // namespace

// Local CLI fast-boot entry. It exercises the actual S3 GPIO48 pad model,
// bypassing Arduino/IDF startup just as the S3 board-level smoke fixtures do.
extern "C" __attribute__((used, noinline)) void labwired_entry() {
  auto *enable1W1ts = reinterpret_cast<volatile uint32_t *>(kGpioBase + 0x30);
  auto *out1W1tc = reinterpret_cast<volatile uint32_t *>(kGpioBase + 0x18);
  auto *out1W1ts = reinterpret_cast<volatile uint32_t *>(kGpioBase + 0x14);
  *enable1W1ts = kGpio48BankBit;
  *out1W1tc = kGpio48BankBit;
  *out1W1ts = kGpio48BankBit;
  for (;;) {
    asm volatile("nop");
  }
}

// Copy-paste Arduino behavior, also run by the hosted verifier against the
// Nano ESP32 catalog board and its USB-Serial-JTAG serial console.
void setup() {
  Serial.begin(115200);
  // Use the SoC GPIO API so this is independent of the Nano core's selectable
  // Arduino-pin versus GPIO-number remapping.
  gpio_set_direction(GPIO_NUM_48, GPIO_MODE_OUTPUT);
  gpio_set_level(GPIO_NUM_48, 1);
  Serial.println("NANO_ESP32_GPIO48_OK");
}

void loop() { delay(100); }
