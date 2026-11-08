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
#  if defined(ARDUINO_ESP32S2_DEV)
// arduino-esp32's A0 is a `static const uint8_t`, not a macro, so the
// `defined(A0)` test below misses it and would read GPIO0 (not an ADC pin on
// the S2: analogRead returns 0 without converting). A0 = GPIO1 = ADC1_CH0.
#    define LW_ADC_PIN A0
#    define LW_ADC_RES_BITS 12
#  elif defined(ARDUINO_ESP32S3_DEV)
// Same `defined(A0)` trap on the S3: GPIO0 is not an ADC pin there either,
// so analogRead(0) returned 0 unconverted and the 0 == 0 pair passed with no
// conversion. A0 = GPIO1 = ADC1_CH0, read through the RTC SAR (SENS).
#    define LW_ADC_PIN A0
#  elif defined(ARDUINO_SAMD_ZERO)
// ArduinoCore-samd's A0 is a `static const uint8_t` as well. analogRead(0)
// would still land on A0 (the core adds A0 to pins below it), but name it:
// A0 = PA02 = AIN0, the input the matrix board drives.
#    define LW_ADC_PIN A0
#  elif defined(A0)
#    define LW_ADC_PIN A0
#  else
#    define LW_ADC_PIN 0
#  endif
#endif

static void logBegin() {
  Serial.begin(115200);
#if defined(ARDUINO_ARCH_RP2040) && !defined(ARDUINO_ARCH_MBED)
  // arduino-pico (Pico 2): Serial is native USB CDC and drops writes until
  // the host opens the port (DTR), on silicon and on the twin alike.
  while (!Serial) delay(1);
#endif
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

#elif defined(ARDUINO_ESP32S2_DEV)
  // esp32s2_sens mid-scale 0x1000 (13-bit) -> 2048 at 12 bits. Exact; a 0
  // here means analogRead never converted.
  return v0 == 2048 && v1 == 2048;

#elif defined(ARDUINO_ESP32S3_DEV)
  // esp32s3_sens mid-scale 0x800 (12-bit). Exact; a 0 here means analogRead
  // never converted.
  return v0 == 2048 && v1 == 2048;

#elif defined(ARDUINO_SAMD_ZERO)
  // samd21_adc: the matrix system drives AIN0 at 1650 mV. AR_DEFAULT is GAIN
  // DIV2 against INTVCC1 (VDDANA / 2), so full scale is 3.3 V and the 10-bit
  // code is 512. Exact; a 0 here means no conversion, or the pad was never
  // handed to the ADC.
  return v0 == 512 && v1 == 512;

#elif defined(ARDUINO_UNOR4_WIFI)
  // ra_adc12 (adc14): the matrix system ties A0 (P014 = AN009) to the 3V3
  // pin. ADC140 runs at 14 bits against the 5 V AVCC0, and the core maps
  // to 10 bits: 3.3 V / 5 V * 1024 = 675. Exact; a 0 here means no
  // conversion.
  return v0 == 675 && v1 == 675;

#elif defined(ARDUINO_TEENSY41)
  // imxrt_adc: the matrix system holds A0 (AD_B1_02 = ADC1_IN7) at 1650 mV.
  // Teensyduino's analog_init runs ADC1 at 10 bits against 3.3 V, so the code
  // is 512. Exact; a 0 here means no conversion or the wrong channel.
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
#ifdef LW_ADC_RES_BITS
  // The S2's RTC SAR returns 13-bit codes; ask the core for 12 like portable
  // sketches do (mapResolution shifts 13 -> 12).
  analogReadResolution(LW_ADC_RES_BITS);
#endif

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
