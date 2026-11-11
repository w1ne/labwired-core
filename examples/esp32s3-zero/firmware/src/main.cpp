// Waveshare ESP32-S3-Zero FH4R2 onboard WS2812 smoke.
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#include <Arduino.h>
#include <esp_cpu.h>
#include <esp_rom_sys.h>
#include <soc/gpio_reg.h>
#include <soc/soc.h>

namespace {
constexpr gpio_num_t kPixelPin = GPIO_NUM_21;
constexpr uint32_t kPixelMask = 1u << 21;
constexpr uint32_t kBitCycles = 300; // 1.25 us at 240 MHz.

void waitUntil(uint32_t start, uint32_t cycles) {
  while (static_cast<uint32_t>(esp_cpu_get_cycle_count() - start) < cycles) {
    asm volatile("nop");
  }
}

void sendBit(bool one) {
  const uint32_t start = esp_cpu_get_cycle_count();
  REG_WRITE(GPIO_OUT_W1TS_REG, kPixelMask);
  waitUntil(start, one ? 168 : 84); // WS2812 T1H ~0.7 us; T0H ~0.35 us.
  REG_WRITE(GPIO_OUT_W1TC_REG, kPixelMask);
  waitUntil(start, kBitCycles);
}

void sendByte(uint8_t value) {
  for (int bit = 7; bit >= 0; --bit) {
    sendBit((value & (1u << bit)) != 0);
  }
}

void sendGreenPixel() {
  // WS2812 transmits GRB, most-significant bit first.
  sendByte(64);
  sendByte(0);
  sendByte(0);
}
} // namespace

void setup() {
  Serial0.begin(115200);
  pinMode(kPixelPin, OUTPUT);
  digitalWrite(kPixelPin, LOW);

  sendGreenPixel();
  esp_rom_delay_us(80); // WS2812 reset/latch low interval.
  sendGreenPixel(); // The next rising edge latches one complete green frame.

  Serial0.println("ESP32_S3_ZERO_GPIO21_WS2812_OK");
}

void loop() { delay(1000); }
