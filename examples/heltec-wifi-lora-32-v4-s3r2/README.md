# Heltec WiFi LoRa 32 V4.3 (ESP32-S3R2)

**Demand rank:** ESP32 is rank **#1** in the standing MCU survey (38%). The
survey does not rank this exact board. This adds board coverage in the highest
demand family; it is not a new chip. Heltec's current product listing reports
the selected S3R2/OLED V4.3 variant in stock at $19.90; see
[`evidence/vendor-status.md`](evidence/vendor-status.md).

This profile boots ESP32-S3 firmware, toggles the board's onboard white status
LED on GPIO35, and attaches the SX1262 model to GP-SPI3. A second smoke checks
LoRa packet-type configuration and the model's TxDone IRQ on DIO1/GPIO14. This
is board-peripheral coverage in ESP32's rank #1 demand family, not a new MCU
pick. Current vendor status and hardware sources are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

The profile does not claim the SSD1315 OLED, RF transmission or reception,
antenna/FEM behavior, PSRAM, GNSS connector, battery or charger, USB, Wi-Fi, or
BLE. The SX1262 twin models its SPI command interface and an immediate TxDone
event, not an over-the-air radio link.

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

## Run the SX1262 command smoke

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id heltec-wifi-lora-32-v4-s3r2-sx1262 \
  --firmware-build-script examples/heltec-wifi-lora-32-v4-s3r2/ci/build-radio-firmware.sh \
  --script examples/heltec-wifi-lora-32-v4-s3r2/radio-onboarding-smoke.yaml \
  --system configs/systems/heltec-wifi-lora-32-v4-s3r2.yaml \
  --out-dir examples/heltec-wifi-lora-32-v4-s3r2/evidence/sx1262
```

This checks `SetPacketType(LoRa)` / `GetPacketType` and the model's `TxDone`
IRQ on DIO1. It does not transmit a packet or verify RF behavior. The run log
and result are committed under `evidence/sx1262/`.
