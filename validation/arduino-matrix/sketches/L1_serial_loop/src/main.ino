// LabWired Arduino matrix L1 — prove loop() + delay/millis scheduling.
void setup() {
  Serial.begin(115200);
#if (defined(ARDUINO_ARCH_RP2040) && !defined(ARDUINO_ARCH_MBED)) || defined(__AVR_ATmega32U4__)
  // arduino-pico (Pico 2) and the ATmega32U4 (Leonardo): Serial is native USB
  // CDC and drops writes until the host opens the port (DTR), on silicon and
  // on the twin alike.
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
