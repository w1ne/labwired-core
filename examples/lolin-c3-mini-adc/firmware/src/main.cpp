// WEMOS LOLIN C3 Mini V2.1.0 GPIO0 / ADC1_CH0 smoke.
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#include <Arduino.h>

namespace {
constexpr uint8_t kAdcPin = 0; // GPIO0 = ADC1_CH0 on ESP32-C3.
constexpr int kMinimumMidpointSample = 1900;
constexpr int kMaximumMidpointSample = 2200;
} // namespace

void setup() {
  Serial.begin(115200);
  analogReadResolution(12);
  analogSetPinAttenuation(kAdcPin, ADC_11db);
  delay(100);

  const int raw = analogRead(kAdcPin);
  if (raw >= kMinimumMidpointSample && raw <= kMaximumMidpointSample) {
    Serial.printf("LOLIN_C3_ADC_OK GPIO0=%d\n", raw);
  } else {
    Serial.printf("LOLIN_C3_ADC_FAIL GPIO0=%d\n", raw);
  }
}

void loop() { delay(1000); }
