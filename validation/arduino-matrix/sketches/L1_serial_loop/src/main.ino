// LabWired Arduino matrix L1 — prove loop() + delay/millis scheduling.
#if defined(ARDUINO_ARDUINO_NANO33BLE) || defined(ARDUINO_NANO33BLE)
#define LW_SERIAL Serial1
#else
#define LW_SERIAL Serial
#endif

void setup() {
  LW_SERIAL.begin(115200);
  delay(1);
  LW_SERIAL.println("LW_L1_BOOT");
}

void loop() {
  delay(1);
  LW_SERIAL.println("LW_L1_OK");
}
