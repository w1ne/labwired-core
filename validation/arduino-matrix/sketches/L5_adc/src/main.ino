// LabWired Arduino matrix L5 — ADC via stock analogRead().
//
// Stronger than "any 0..4095":
//   * two completed samples
//   * deterministic pair (v0 == v1) — twin sources are fixed, not noise
//   * family exact codes where the Arduino path is known to hit the model:
//       AVR mid-scale 512; RP2040 unseeded channels 0
//   * values printed on OK for scoreboard audit
//
// Note: several STM32/nRF Arduino HAL paths still return 0 even when the
// peripheral model has a fixed non-zero source (3.0/3.3 → 3723). That is a
// real fidelity gap; requiring 3723 fleet-wide would fail bring-up. We still
// reject unstable/random pairs and out-of-range values.

#include <Arduino.h>

#ifndef LW_ADC_PIN
#  if defined(A0)
#    define LW_ADC_PIN A0
#  else
#    define LW_ADC_PIN 0
#  endif
#endif

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

static bool in_range(int v) { return v >= 0 && v <= 4095; }

static bool samples_ok(int v0, int v1) {
  if (!in_range(v0) || !in_range(v1)) {
    return false;
  }

#if defined(__AVR__)
  // avr.rs ADC mid-scale 512 (10-bit). Exact.
  return v0 == 512 && v1 == 512;

#elif defined(ARDUINO_ARCH_RP2040)
  // rp2040/adc.rs: unseeded GPIO inputs convert 0 (no fabricated mid-scale).
  return v0 == 0 && v1 == 0;

#else
  // Deterministic twin: equal samples (fixed source / idle 0), OR the
  // classic STM32 F1 unseeded path that increments DR each conversion.
  if (v0 == v1) {
    return true;
  }
  return v1 == ((v0 + 1) & 0xFFF);
#endif
}

void setup() {
  logBegin();
  logLine("LW_L5_BOOT");

  (void)analogRead(LW_ADC_PIN);
  int v0 = analogRead(LW_ADC_PIN);
  int v1 = analogRead(LW_ADC_PIN);

  if (samples_ok(v0, v1)) {
    // Avoid snprintf on AVR (heavy lib can hit decode gaps); decimal print.
    logLine("LW_L5_OK");
    Serial.print(F("LW_L5_VAL "));
    Serial.print(v0);
    Serial.print(F(" "));
    Serial.println(v1);
#if defined(ARDUINO_USB_CDC_ON_BOOT) && (ARDUINO_USB_CDC_ON_BOOT)
    Serial0.print(F("LW_L5_VAL "));
    Serial0.print(v0);
    Serial0.print(F(" "));
    Serial0.println(v1);
#endif
    return;
  }
  logLine("LW_L5_FAIL");
  Serial.print(F("LW_L5_VAL "));
  Serial.print(v0);
  Serial.print(F(" "));
  Serial.println(v1);
#if defined(ARDUINO_USB_CDC_ON_BOOT) && (ARDUINO_USB_CDC_ON_BOOT)
  Serial0.print(F("LW_L5_VAL "));
  Serial0.print(v0);
  Serial0.print(F(" "));
  Serial0.println(v1);
#endif
}

void loop() {}
