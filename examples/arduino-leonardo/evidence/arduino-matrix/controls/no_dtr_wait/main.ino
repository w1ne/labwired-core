// Negative control: stock L0 without while(!Serial). Leonardo CDC drops
// writes until the host sets DTR, so nothing should reach the console.
void setup() {
  Serial.begin(115200);
  delay(1);
  Serial.println("LW_L0_OK");
}
void loop() {}
