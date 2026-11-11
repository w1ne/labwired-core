# Adafruit ESP32 Feather V2

**Demand rank:** ESP32 family, rank 1 (38% of respondents in the measured
embedded MCU survey). This adds a board-level smoke for an in-stock ESP32
Feather with STEMMA QT; the existing classic ESP32 model and Feather pin map
are reused. Live purchase and pin evidence is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Copy-paste Arduino sketch

This turns on the board's red user LED (`LED_BUILTIN`, GPIO13):

```cpp
#include <Arduino.h>

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  digitalWrite(LED_BUILTIN, HIGH);
}

void loop() { delay(100); }
```

The exact catalog board and sketch were model-verified against a GPIO13-high
oracle. The hosted profile does not capture this board's `Serial` output, so
the example asserts the LED state instead of presenting serial output as
evidence.

## Local model smoke

Run the committed local fixture with:

```sh
scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-esp32-v2-gpio \
  --script examples/adafruit-feather-esp32-v2/io-smoke.yaml \
  --system configs/systems/adafruit-feather-esp32-v2.yaml \
  --out-dir examples/adafruit-feather-esp32-v2/evidence/run \
  --firmware-build-script examples/adafruit-feather-esp32-v2/build-firmware.sh
```

PlatformIO builds the Arduino sketch for the exact `adafruit_feather_esp32_v2`
target. For the local CLI smoke, its ELF entry is redirected to a tiny
GPIO13-only fast-boot routine that bypasses Arduino/ESP-IDF startup. The hosted
verifier separately boots the copy-paste sketch on the catalog board. The
committed local run log and structured metrics are in `evidence/run/`.

## Scope

The system profile claims the classic ESP32 GPIO model and the Feather's red
GPIO13 user LED. The local smoke checks GPIO13 low-to-high pad transitions. It
does not claim Wi-Fi/Bluetooth, PSRAM, the NeoPixel on GPIO0, the GPIO2
STEMMA-QT/NeoPixel power switch, or I2C sensor behavior. The hosted verifier
reports an ideal-3.3-V power approximation; power-on and brown-out behavior are
not simulated. No physical board was tested.
