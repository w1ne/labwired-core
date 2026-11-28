// LabWired Arduino matrix L0 — prove setup() + Serial after core boot.
void setup() {
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
  // Minimal settle; sim UART is ready immediately (was delay(10) spin tax).
  delay(1);
  Serial.println("LW_L0_OK");
}

void loop() {
  // Idle. Marker is only required once from setup().
}
