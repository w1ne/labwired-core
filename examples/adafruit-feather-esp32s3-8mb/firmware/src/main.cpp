// LabWired - Adafruit ESP32-S3 Feather 8MB/no-PSRAM smoke
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#include <Arduino.h>
#include <esp_cpu.h>
#include <esp_rom_sys.h>
#include <soc/gpio_reg.h>
#include <soc/soc.h>

#ifndef PIN_NEOPIXEL
#define PIN_NEOPIXEL 33
#endif
#ifndef NEOPIXEL_POWER
#define NEOPIXEL_POWER 21
#endif

namespace {
constexpr uint32_t kPixelMask = 1u << (PIN_NEOPIXEL - 32);
constexpr uint32_t kBitCycles = 300; // 1.25 us at the Feather's 240 MHz clock.

void waitUntil(uint32_t start, uint32_t cycles) {
  while (static_cast<uint32_t>(esp_cpu_get_cycle_count() - start) < cycles) {
    asm volatile("nop");
  }
}

void sendBit(bool one) {
  const uint32_t start = esp_cpu_get_cycle_count();
  REG_WRITE(GPIO_OUT1_W1TS_REG, kPixelMask);
  waitUntil(start, one ? 168 : 84); // WS2812 T1H ~0.7 us; T0H ~0.35 us.
  REG_WRITE(GPIO_OUT1_W1TC_REG, kPixelMask);
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
}

void setup() {
  Serial0.begin(115200);

  // Match the Feather variant's built-in NeoPixel power gate before using it.
  pinMode(NEOPIXEL_POWER, OUTPUT);
  digitalWrite(NEOPIXEL_POWER, HIGH);
  pinMode(PIN_NEOPIXEL, OUTPUT);
  digitalWrite(PIN_NEOPIXEL, LOW);
  sendGreenPixel();
  esp_rom_delay_us(80); // WS2812 reset/latch low interval.
  sendGreenPixel(); // The next rising edge latches the first complete frame.

  Serial0.println("FEATHER_S3_GPIO33_WS2812_OK");
}

void loop() { delay(1000); }
