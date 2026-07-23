// LabWired Arduino matrix L0 — prove setup() + Serial after core boot.
void setup() {
  Serial.begin(115200);
  // Minimal settle; sim UART is ready immediately (was delay(10) spin tax).
  delay(1);
  Serial.println("LW_L0_OK");
}

void loop() {
  // Idle. Marker is only required once from setup().
}
