// Seeed Studio XIAO ESP32-S3 USER_LED + USB Serial/JTAG smoke.
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#include <Arduino.h>

namespace {
constexpr int kUserLedPin = 21; // XIAO user LED, active low
}

void setup() {
  Serial.begin(115200);
  pinMode(kUserLedPin, OUTPUT);
  digitalWrite(kUserLedPin, HIGH); // off

  for (int i = 0; i < 3; ++i) {
    digitalWrite(kUserLedPin, LOW); // on
    delay(10);
    digitalWrite(kUserLedPin, HIGH); // off
    delay(10);
  }
  digitalWrite(kUserLedPin, LOW); // leave the indicator on after the smoke
  Serial.println("XIAO_ESP32S3_LED_OK GPIO21_ACTIVE_LOW");
}

void loop() { delay(1000); }
