// Probe: earlephilhower Serial (USB CDC) on Pico 2 / RP2350 (no CYW43).
// Built: arduino-cli --fqbn rp2040:rp2040:rpipico2 (arduino-pico 6.3.2)
// Pico 2 W's initVariant() calls init_cyw43_wifi(); that path is covered by
// the Pico 2 W board onboarding (Wi-Fi unmodelled + cyw43 stub), not this gate.
// Print from loop() — host enum + CDC DTR finishes after Serial.begin() returns.
void setup() {
  Serial.begin(115200);
}
void loop() {
  if (Serial) {
    Serial.println("RP2350_USB_CDC_OK");
    delay(50);
  }
}
