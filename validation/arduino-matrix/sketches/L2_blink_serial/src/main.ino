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
#if defined(ARDUINO_ARCH_RP2040) && !defined(ARDUINO_ARCH_MBED)
  // arduino-pico (Pico 2): Serial is native USB CDC and drops writes until
  // the host opens the port (DTR), on silicon and on the twin alike.
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
