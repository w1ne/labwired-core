// LabWired Arduino matrix L6 — PWM via stock analogWrite().
//
// Stronger than "didn't hang": leave a mid duty running long enough for the
// matrix runner's optional --watch-gpio / RMT oracle to observe edges, then
// print OK while the timer/LEDC is still active (not after duty 0).

#include <Arduino.h>

#ifndef LW_PWM_PIN
  // Nucleo-G474RE LED is PA5 = DAC1_OUT2 — analogWrite prefers DAC and the
  // twin has no DAC window → memory_violation. Use TIM2_CH1 PA0.
#  if defined(ARDUINO_NUCLEO_G474RE) || defined(ARDUINO_NUCLEO_G474RE_P)
#    define LW_PWM_PIN PA0
#  elif defined(ARDUINO_NUCLEO_G071RB)
// NUCLEO-G071RB LD4 is PA5 = DAC1_OUT2 too, and the twin models DAC1, so
// analogWrite(LED_BUILTIN) would pass on the DAC. Use TIM2_CH1 on PA0.
#    define LW_PWM_PIN PA0
#  elif defined(STM32U545xx)
// NUCLEO-U545RE-Q LD2 is PA5 = DAC1_OUT2 as well; use TIM2_CH1 on PA0.
#    define LW_PWM_PIN PA0
#  elif defined(STM32L073xx) || defined(STM32L476xx)
// NUCLEO-L073RZ / NUCLEO-L476RG LD2 is PA5 = DAC1_OUT2 and the twin models
// the DAC, so analogWrite(LED_BUILTIN) would pass on the DAC. Use TIM2_CH1
// on PA0 (AF2 on the L0, AF1 on the L4).
#    define LW_PWM_PIN PA0
#  elif defined(STM32WB55xx)
// P-NUCLEO-WB55 LD2 is PB5, which has no timer channel in stm32duino's
// PinMap_TIM, so analogWrite would fall back to digitalWrite. Use TIM2_CH1
// on PA0.
#    define LW_PWM_PIN PA0
#  elif defined(STM32H563xx)
// NUCLEO-H563ZI LD1 is PB0, whose PinMap_TIM entry is TIM1_CH2N, a
// complementary output the twin's timer does not generate. Use TIM2_CH1 on
// PA0.
#    define LW_PWM_PIN PA0
#  elif defined(ARDUINO_BLACKPILL_F411CE)
// WeAct Black Pill LED is PC13, which has no timer channel on F411, so
// analogWrite would fall back to digitalWrite. Use TIM2_CH1 on PA0.
#    define LW_PWM_PIN PA0
#  elif defined(ARDUINO_NUCLEO_G0B1RE)
// NUCLEO-G0B1RE LD4 is PA5 = DAC1_OUT2 and the twin has no DAC1, so
// analogWrite(LED_BUILTIN) would take the DAC path. Use TIM2_CH1 on PA0.
#    define LW_PWM_PIN PA0
#  elif defined(CONFIG_IDF_TARGET_ESP32C6) || defined(CONFIG_IDF_TARGET_ESP32C3) || \
      defined(CONFIG_IDF_TARGET_ESP32S3)
// The C6/C3/S3 DevKit LED_BUILTIN is a WS2812 (GPIO8/GPIO8/GPIO48) driven by
// RMT, so analogWrite on it never exercises the LEDC. Use plain GPIO2.
#    define LW_PWM_PIN 2
#  elif defined(ARDUINO_BLUEPILL_F103C8)
// Blue Pill LED is PC13, which has no timer channel on F103, so analogWrite
// would fall back to digitalWrite. Use TIM2_CH1 on PA0 (no remap).
#    define LW_PWM_PIN PA0
#  elif defined(ARDUINO_ARCH_AVR) && defined(__AVR_ATmega328P__)
// Uno/Nano LED_BUILTIN is D13 = PB5, which has no timer output on the
// ATmega328P, so analogWrite(13, 128) is digitalWrite(HIGH). Use D9 = PB1 =
// OC1A (Timer1, phase-correct 8-bit, 16 MHz / 64 / 510 = 490 Hz).
#    define LW_PWM_PIN 9
#  elif defined(LED_BUILTIN)
#    define LW_PWM_PIN LED_BUILTIN
#  elif defined(ARDUINO_ARCH_ESP32)
#    define LW_PWM_PIN 2
#  elif defined(ARDUINO_ARCH_RP2040)
#    define LW_PWM_PIN 25
#  else
#    define LW_PWM_PIN 9
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

void setup() {
  logBegin();
  logLine("LW_L6_BOOT");

  pinMode(LW_PWM_PIN, OUTPUT);
  // Mid duty — leave running so logic-edge / RMT oracles can fire.
  analogWrite(LW_PWM_PIN, 128);
  // Enough wall time for timer PWM edges under typical matrix step budgets.
  for (int i = 0; i < 30; i++) {
    delay(1);
  }

  logLine("LW_L6_OK");
  // Intentionally do not analogWrite(0) before the marker — host may still be
  // sampling edges for a few settle steps after uart_contains matches.
}

void loop() {
  // Keep PWM alive if the sim continues past the marker settle window.
  delay(1);
}
