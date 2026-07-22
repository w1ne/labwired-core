// LabWired Arduino matrix L1 — prove loop() runs and millis advances.
void setup() {
  Serial.begin(115200);
  delay(10);
  Serial.println("LW_L1_BOOT");
}

void loop() {
  Serial.println("LW_L1_OK");
  delay(50);
}
