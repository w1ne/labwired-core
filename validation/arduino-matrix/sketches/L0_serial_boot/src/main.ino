// LabWired Arduino matrix L0 — prove setup() + Serial after core boot.
void setup() {
  Serial.begin(115200);
#if defined(ARDUINO_ARCH_RP2040) && !defined(ARDUINO_ARCH_MBED)
  // arduino-pico (Pico 2): Serial is native USB CDC and drops writes until
  // the host opens the port (DTR), on silicon and on the twin alike.
  while (!Serial) delay(1);
#endif
  // Minimal settle; sim UART is ready immediately (was delay(10) spin tax).
  delay(1);
  Serial.println("LW_L0_OK");
}

void loop() {
  // Idle. Marker is only required once from setup().
}
