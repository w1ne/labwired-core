// LabWired Arduino matrix L1 — prove loop() + delay/millis scheduling.
void setup() {
  Serial.begin(115200);
  delay(1);
  Serial.println("LW_L1_BOOT");
}

void loop() {
  // Short delay still exercises millis/SysTick/RTOS tick paths without
  // dominating wall time (was delay(50)).
  delay(1);
  Serial.println("LW_L1_OK");
}
