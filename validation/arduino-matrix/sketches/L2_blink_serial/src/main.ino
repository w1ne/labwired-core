// LabWired Arduino matrix L2 — LED_BUILTIN digitalWrite + serial marker.
// Pin: LED_BUILTIN when defined, else board-common fallbacks.
#if defined(ARDUINO_NANO_ESP32)
// The Nano ESP32 variant declares LED_BUILTIN as a constexpr, not a macro, so
// the #ifndef below cannot see it and would pick the ESP32 fallback (pin 2).
// LED_BUILTIN = D13; the core remaps it to GPIO48.
#  define LW_LED LED_BUILTIN
#elif !defined(LED_BUILTIN)
#  if defined(ARDUINO_ARCH_ESP32)
#    define LW_LED 2
#  elif defined(ARDUINO_ARCH_RP2040)
#    define LW_LED 25
#  elif defined(ARDUINO_ARCH_NRF52) || defined(ARDUINO_ARCH_NRF52840)
#    define LW_LED 13
#  else
#    define LW_LED 13
#  endif
#else
#  define LW_LED LED_BUILTIN
#endif

void setup() {
  pinMode(LW_LED, OUTPUT);
  Serial.begin(115200);
#if (defined(ARDUINO_ARCH_RP2040) && !defined(ARDUINO_ARCH_MBED)) || defined(__AVR_ATmega32U4__) || \
    defined(ARDUINO_SAMD_NANO_33_IOT) || \
    ((defined(CONFIG_IDF_TARGET_ESP32S3) || defined(CONFIG_IDF_TARGET_ESP32S2)) && \
     defined(ARDUINO_USB_CDC_ON_BOOT) && ARDUINO_USB_CDC_ON_BOOT && !ARDUINO_USB_MODE)
  // arduino-pico (Pico 2) and the ATmega32U4 (Leonardo): Serial is native USB
  // CDC and drops writes until the host opens the port (DTR), on silicon and
  // on the twin alike.
  // ESP32-S2/S3 with ARDUINO_USB_MODE=0 (Arduino Nano ESP32): Serial is
  // TinyUSB CDC on USB-OTG, which drops writes until DTR the same way.
  // ArduinoCore-samd on the Nano 33 IoT: Serial is SerialUSB, native USB CDC,
  // and operator bool() is false until the host raises DTR.
  while (!Serial) delay(1);
#endif
  // Short delays: matrix stops on first LW_L2_OK; long wall time was almost
  // entirely delay() spin, not model work. 1 ms still exercises digitalWrite
  // + millis/SysTick scheduling without 40+ ms of empty wait per loop.
  delay(1);
  Serial.println("LW_L2_BOOT");
}

void loop() {
  digitalWrite(LW_LED, HIGH);
  delay(1);
  digitalWrite(LW_LED, LOW);
  delay(1);
  Serial.println("LW_L2_OK");
}
