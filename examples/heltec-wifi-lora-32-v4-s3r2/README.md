# Heltec WiFi LoRa 32 V4.3 (ESP32-S3R2)

**Demand rank:** ESP32 is rank **#1** in the standing MCU survey (38%). The
survey does not rank this exact board. This adds board coverage in the highest
demand family; it is not a new chip. Heltec's current product listing reports
the selected S3R2/OLED V4.3 variant in stock at $19.90; see
[`evidence/vendor-status.md`](evidence/vendor-status.md).

This narrow profile boots ESP32-S3 firmware and toggles the board's onboard white
status LED on GPIO35. The LED's GPIO model is checked by the committed local
smoke. The profile does not claim the SSD1315 OLED, SX1262 LoRa radio, PSRAM,
GNSS connector, battery or charger, USB, Wi-Fi, or BLE.

## Copy-paste Arduino example

Select **Heltec WiFi LoRa 32 (V4)** in Arduino IDE, then upload:

```cpp
constexpr int STATUS_LED = 35;

void setup() {
  pinMode(STATUS_LED, OUTPUT);
}

void loop() {
  digitalWrite(STATUS_LED, HIGH);
  delay(500);
  digitalWrite(STATUS_LED, LOW);
  delay(500);
}
```

The board family includes both S3R2 and S3R8 options. This example and the
smoke target the S3R2 V4.3 variant, whose status LED is on GPIO35.

## Run the local simulator smoke

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id heltec-wifi-lora-32-v4-s3r2 \
  --firmware-build-script examples/heltec-wifi-lora-32-v4-s3r2/ci/build-firmware.sh \
  --script examples/heltec-wifi-lora-32-v4-s3r2/onboarding-smoke.yaml \
  --system configs/systems/heltec-wifi-lora-32-v4-s3r2.yaml \
  --out-dir examples/heltec-wifi-lora-32-v4-s3r2/evidence
```

The smoke asserts repeated GPIO35 transitions. It does not test the physical
board or any of the omitted components. See
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
