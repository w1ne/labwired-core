void setup() {
  pinMode(PA5, OUTPUT);
  Serial.begin(115200);
  digitalWrite(PA5, HIGH);
  Serial.println("NUCLEO_G071RB_OK");
}

void loop() {}
