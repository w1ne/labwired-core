// LabWired Arduino matrix L2 — LED_BUILTIN digitalWrite + serial marker.
#if defined(ARDUINO_ARDUINO_NANO33BLE) || defined(ARDUINO_NANO33BLE)
#define LW_SERIAL Serial1
#else
#define LW_SERIAL Serial
#endif

#ifndef LED_BUILTIN
#  if defined(ARDUINO_ARCH_ESP32)
#    define LW_LED 2
#  elif defined(ARDUINO_ARCH_RP2040)
#    define LW_LED 25
#  elif defined(ARDUINO_ARCH_NRF52) || defined(ARDUINO_ARCH_NRF52840) || defined(ARDUINO_ARDUINO_NANO33BLE)
#    define LW_LED 13
#  else
#    define LW_LED 13
#  endif
#else
#  define LW_LED LED_BUILTIN
#endif

void setup() {
  pinMode(LW_LED, OUTPUT);
  LW_SERIAL.begin(115200);
  delay(1);
  LW_SERIAL.println("LW_L2_BOOT");
}

void loop() {
  digitalWrite(LW_LED, HIGH);
  delay(1);
  digitalWrite(LW_LED, LOW);
  delay(1);
  LW_SERIAL.println("LW_L2_OK");
}
