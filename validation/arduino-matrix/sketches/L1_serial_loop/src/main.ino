// LabWired Arduino matrix L1 — prove loop() + delay/millis scheduling.
void setup() {
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
  delay(1);
  Serial.println("LW_L1_BOOT");
}

void loop() {
  // Short delay still exercises millis/SysTick/RTOS tick paths without
  // dominating wall time (was delay(50)).
  delay(1);
  Serial.println("LW_L1_OK");
}
