# M5Stack Cardputer-Adv (SKU K132-ADV)

**Demand rank:** ESP32 is rank **#1** in the standing MCU survey (38%). The
survey does not rank this exact board. This adds board coverage in the highest
demand family; it is not a new chip. M5Stack's live product listing showed the
Cardputer-Adv in stock on 2026-11-06; see
[`evidence/vendor-status.md`](evidence/vendor-status.md).

This narrow profile boots ESP32-S3 firmware, initializes the integrated
ST7789V2 display through GP-SPI3, draws an 8×8 red patch, turns on the
backlight, and reports a UART0 marker. It uses the existing ESP32-S3 and
ST7789 models. The profile does not claim support for PSRAM, the keyboard, IMU,
audio, SD slot, infrared transmitter, battery, radio, or expansion ports.

## Copy-paste Arduino example

Install M5Stack's M5Cardputer library and upload this sketch to a Cardputer-Adv:

```cpp
#include <M5Cardputer.h>

void setup() {
  auto config = M5.config();
  M5Cardputer.begin(config);
  M5Cardputer.Display.fillScreen(TFT_BLACK);
  M5Cardputer.Display.fillRect(0, 0, 16, 16, TFT_RED);
}

void loop() {}
```

The sketch uses the board library to configure the display and paints the same
red square used by the simulator smoke. It is a usage example, not a second
firmware image built as part of the simulator run.

## Run the local simulator smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id m5stack-cardputer-adv \
  --firmware-build-script examples/m5stack-cardputer-adv/ci/build-firmware.sh \
  --script examples/m5stack-cardputer-adv/onboarding-smoke.yaml \
  --system configs/systems/m5stack-cardputer-adv.yaml \
  --out-dir examples/m5stack-cardputer-adv/evidence
```

The smoke assertion checks the UART marker and a lit display region with 64 red
pixels. Build logs, raw simulation output, and metrics are committed under
[`evidence/`](evidence/). This is local simulator evidence, not a physical-board
test or a hosted `model_verified` result. See
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
