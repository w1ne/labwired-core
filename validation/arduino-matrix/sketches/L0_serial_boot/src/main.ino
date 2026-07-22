// LabWired Arduino matrix L0 — prove setup() + Serial after core boot.
void setup() {
  Serial.begin(115200);
  // Some cores need a short wait for USB CDC; sim UART is ready immediately.
  delay(10);
  Serial.println("LW_L0_OK");
}

void loop() {
  // Idle. Marker is only required once from setup().
}
