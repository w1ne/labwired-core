// LabWired Arduino matrix L2 — LED_BUILTIN digitalWrite + serial marker.
// Pin: LED_BUILTIN when defined, else board-common fallbacks.
#ifndef LED_BUILTIN
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
  delay(10);
  Serial.println("LW_L2_BOOT");
}

void loop() {
  digitalWrite(LW_LED, HIGH);
  delay(20);
  digitalWrite(LW_LED, LOW);
  delay(20);
  Serial.println("LW_L2_OK");
}
