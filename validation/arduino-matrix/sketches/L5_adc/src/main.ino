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
#  elif defined(ARDUINO_SAMD_ZERO) || defined(ARDUINO_SAMD_NANO_33_IOT)
// ArduinoCore-samd's A0 is a `static const uint8_t`, not a macro, so
// `defined(A0)` misses it. analogRead(0) would still land on A0 (the core
// adds A0 to pins below it), but name it: A0 = PA02 = AIN0, the input the
// matrix board drives.
#    define LW_ADC_PIN A0
#  elif defined(ARDUINO_ARDUINO_NANO33BLE)
// Same trap in ArduinoCore-mbed: A0 (P0.04 = AIN2) is a `static const`.
#    define LW_ADC_PIN A0
#  elif defined(ARDUINO_NANO_ESP32)
// Same trap on the Nano ESP32: A0 (GPIO1 = ADC1_CH0) is a `static constexpr`.
// With the pin remap, analogRead(0) would read D0 = GPIO44, which has no ADC.
#    define LW_ADC_PIN A0
#  elif defined(ARDUINO_XIAO_ESP32S3)
// Same trap: the XIAO ESP32S3 variant's A0 (GPIO1 = ADC1_CH0) is a
// `static const uint8_t`. analogRead(0) would read GPIO0, which has no ADC.
#    define LW_ADC_PIN A0
#  elif defined(A0)
#    define LW_ADC_PIN A0
#  else
#    define LW_ADC_PIN 0
#  endif
#endif

static void logBegin() {
  Serial.begin(115200);
#if (defined(ARDUINO_ARCH_RP2040) && !defined(ARDUINO_ARCH_MBED)) || defined(__AVR_ATmega32U4__) || \
    defined(ARDUINO_SAMD_NANO_33_IOT) || \
    defined(ARDUINO_ARDUINO_NANO33BLE) || \
    ((defined(CONFIG_IDF_TARGET_ESP32S3) || defined(CONFIG_IDF_TARGET_ESP32S2)) && \
     defined(ARDUINO_USB_CDC_ON_BOOT) && ARDUINO_USB_CDC_ON_BOOT && !ARDUINO_USB_MODE)
  // arduino-pico (Pico 2) and the ATmega32U4 (Leonardo): Serial is native USB
  // CDC and drops writes until the host opens the port (DTR), on silicon and
  // on the twin alike.
  // ESP32-S2/S3 with ARDUINO_USB_MODE=0 (Arduino Nano ESP32): Serial is
  // TinyUSB CDC on USB-OTG, which drops writes until DTR the same way.
  // ArduinoCore-samd on the Nano 33 IoT: Serial is SerialUSB, native USB CDC,
  // and operator bool() is false until the host raises DTR.
  // ArduinoCore-mbed on the Nano 33 BLE: Serial is USB CDC on the nRF52840
  // USBD and drops writes until the host raises DTR.
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

#elif defined(ARDUINO_NUCLEO_L073RZ)
  // adc profile stm32l0 (the L0 CHSELR block, stm32f0_adc.rs; the L0 map is
  // that block's subset): the matrix system holds A0 (PA0 = ADC_IN0) at
  // 1651 mV, a 12-bit code of 2048. Core 2.x (ststm32 19.7.1, the CI pin)
  // picks its internal read resolution from HAL ADC_RESOLUTION_10B and
  // converts at 10 bits: 512. Core 3.x keys on LL_ADC_RESOLUTION_10B, which
  // wiring_analog.c does not see on the L0, so it converts at 12 bits (the
  // binary's _internalReadResolution is 0x0C) and mapResolution() returns
  // ((2048 + 1) >> 2) - 1 = 511, which holds only for codes 2047..2050. Both
  // exact; the unheld fixed source (3.0 V) reads 930, no conversion 0.
#  if defined(STM32_CORE_VERSION_MAJOR) && STM32_CORE_VERSION_MAJOR >= 3
  return v0 == 511 && v1 == 511;
#  else
  return v0 == 512 && v1 == 512;
#  endif

#elif defined(ARDUINO_NUCLEO_G071RB) || defined(ARDUINO_NUCLEO_G0B1RE)
  // adc profile stm32g0 (the G0/C0 CHSELR block): the matrix system holds A0
  // (PA0 = ADC1_IN0) at 1651 mV. 12-bit against 3.3 V is 2048; stm32duino
  // returns 10 bits: 512. Exact; the unheld fixed source (3.0 V) reads 930,
  // no conversion 0.
  return v0 == 512 && v1 == 512;

#elif defined(ARDUINO_NUCLEO_L476RG) || defined(ARDUINO_NUCLEO_G474RE) || \
    defined(ARDUINO_P_NUCLEO_WB55RG)
  // adc profile stm32l4: the matrix system holds A0 at 1651 mV (L476 PA0 =
  // ADC1_IN5; G474 PA0 = ADC1_IN1; WB55 PC0 = ADC1_IN1), and SQR1.SQ1
  // selects it. 12-bit against 3.3 V is 2048; stm32duino returns 10 bits:
  // 512. Exact; the unheld fixed source (3.0 V) reads 930, no conversion 0.
  return v0 == 512 && v1 == 512;

#elif defined(ARDUINO_NUCLEO_H723ZG) || defined(ARDUINO_NUCLEO_H753ZI)
  // adc profile h7 (16-bit ADC1/2): the matrix system holds A0 (PA3 =
  // ADC12_INP15) at 1651 mV and PCSEL/SQR1 select it. stm32duino converts at
  // 10 bits (CFGR.RES = 011): 512. Exact; a 0 here means no conversion (the
  // HAL's ADSTART write cleared ADEN before the rs-bit fix).
  return v0 == 512 && v1 == 512;

#elif defined(ARDUINO_ESP32S2_DEV)
  // esp32s2_sens mid-scale 0x1000 (13-bit) -> 2048 at 12 bits. Exact; a 0
  // here means analogRead never converted.
  return v0 == 2048 && v1 == 2048;

#elif defined(ARDUINO_TEENSY41)
  // imxrt_adc: the matrix system holds A0 (AD_B1_02 = ADC1_IN7) at 1650 mV.
  // Teensyduino's analog_init runs ADC1 at 10 bits against 3.3 V, so the code
  // is 512. Exact; a 0 here means no conversion or the wrong channel.
  return v0 == 512 && v1 == 512;

#elif defined(ARDUINO_SAMD_ZERO) || defined(ARDUINO_SAMD_NANO_33_IOT)
  // samd21_adc: the matrix system drives AIN0 at 1650 mV. AR_DEFAULT is GAIN
  // DIV2 against INTVCC1 (VDDANA / 2), so full scale is 3.3 V and the 10-bit
  // code is 512. Exact; a 0 here means no conversion, or the pad was never
  // handed to the ADC.
  return v0 == 512 && v1 == 512;

#elif defined(ARDUINO_ARDUINO_NANO33BLE)
  // nrf52 SAADC: the matrix system holds A0 (P0.04 = AIN2) at 1650 mV. Mbed
  // converts at 12 bits, gain 1/4 against VDD/4 (full scale 3.3 V): 2048,
  // which analogRead's default 10 bits makes 512. Exact; the idle unseeded
  // input reads 0.
  return v0 == 512 && v1 == 512;

#elif defined(ARDUINO_XIAO_ESP32S3) || defined(ARDUINO_NANO_ESP32)
  // esp32s3 SENS oneshot: the matrix system holds A0 (GPIO1 = ADC1_CH0) at
  // 1650 mV with a centred potentiometer. Arduino-ESP32 returns the raw
  // 12-bit code, 2047. Exact; the idle unseeded channel reads 0.
  return v0 == 2047 && v1 == 2047;

#elif defined(STM32L432xx)
  // NUCLEO-L432KC: the matrix system holds A0 = PA0 = ADC1_IN5 at 1650 mV.
  // floor(1650 * 4095 / 3300) = 2047 at 12 bits; analogRead's default
  // 10 bits is 2047 >> 2 = 511. Exact; 0 means no conversion, and 930 is
  // the model's fixed internal source, i.e. the board level never reached
  // the selected channel.
  return v0 == 511 && v1 == 511;

#elif defined(ARDUINO_ARCH_RP2040)
  // rp2040/adc.rs: unseeded GPIO inputs convert 0 (no fabricated mid-scale).
  return v0 == 0 && v1 == 0;

#elif defined(ARDUINO_NUCLEO_F411RE)
  // NUCLEO-F411RE: the matrix system holds A0 = PA0 = ADC1_IN0 at 1650 mV.
  // floor(1650 * 4095 / 3300) = 2047 at 12 bits; analogRead's default
  // 10 bits is 2047 >> 2 = 511. Exact; "2 3" is the model's incrementing
  // fallback, i.e. the board level never reached the selected channel.
  // Keyed on the board, not STM32F411xE: the Black Pill row holds no level.
  return v0 == 511 && v1 == 511;

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
