# M5Stack AtomS3-Lite (SKU C124)

**Demand rank:** ESP32 is rank 1 in the 2025 JetBrains State of Embedded MCU
survey (38%). This is board-coverage work in the highest-demand family; the
survey does not rank this exact board. The M5Stack store listed SKU C124 in
stock when checked on 2026-11-06. See
[`evidence/vendor-status.md`](evidence/vendor-status.md).

The narrow board profile exposes the ESP32-S3FN8's 8 MiB flash map, GPIO, and
the onboard WS2812C-2020 RGB LED on GPIO35. The firmware sends a red GRB frame
with GPIO timing. The local smoke requires at least 48 GPIO transitions and a
decoded red pixel. It does not cover the button, infrared transmitter, USB,
radio, RMT TX, or GPIO connector peripherals.

## Copy-paste Arduino example

Install the Adafruit NeoPixel library and upload this sketch to the AtomS3-Lite:

```cpp
#include <Adafruit_NeoPixel.h>

constexpr uint8_t RGB_PIN = 35;
Adafruit_NeoPixel led(1, RGB_PIN, NEO_GRB + NEO_KHZ800);

void setup() {
  led.begin();
  led.setBrightness(32);
  led.setPixelColor(0, led.Color(255, 0, 0));
  led.show();
}

void loop() {}
```

M5Stack's pinout assigns the RGB LED to GPIO35 and identifies it as a
WS2812C-2020. The sketch demonstrates the same LED and pin using the Arduino
library.

## Run the local simulator smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id m5stack-atoms3-lite \
  --firmware-build-script examples/m5stack-atoms3-lite/ci/build-firmware.sh \
  --script examples/m5stack-atoms3-lite/onboarding-smoke.yaml \
  --system configs/systems/m5stack-atoms3-lite.yaml \
  --out-dir examples/m5stack-atoms3-lite/evidence
```

Build and simulation logs, raw simulator output, and generated onboarding
metrics are committed under [`evidence/`](evidence/). This is local simulator
evidence, not a physical-board test. The profile's omissions are listed in
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
