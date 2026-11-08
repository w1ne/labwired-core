# ESP32-C6-DevKitM-1-N4

Demand rank: **ESP32 family #1** in the JetBrains 2025 MCU-use survey (38%).
This is a family-depth board extension using the existing C6 models; it does
not add a new MCU core. The exact N4 board was absent from the repo.

The narrow board profile runs a bare-metal HP-core smoke that sends UART0 text
and drives the built-in WS2812B RGB LED on GPIO8 through RMT channel 0. The
descriptor sets the board's actual 4 MB flash size. It does not claim the LP
core, ROM boot, wireless, USB, or unlisted peripherals.

## Copy-paste Arduino example

In Arduino IDE, select **ESP32C6 Dev Module** and install the Adafruit
NeoPixel library. Upload this sketch to turn the built-in LED red:

```cpp
#include <Adafruit_NeoPixel.h>

Adafruit_NeoPixel led(1, 8, NEO_GRB + NEO_KHZ800);

void setup() {
  led.begin();
  led.setPixelColor(0, led.Color(255, 0, 0));
  led.show();
}

void loop() {}
```

This Arduino sketch is a usage example; the committed simulator smoke is the
bare-metal Rust image under `crates/firmware-esp32c6-devkitm1-n4-smoke`.

## Run the repository smoke

From the repository root:

```bash
rustup target add riscv32imc-unknown-none-elf
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c6-devkitm1-n4 \
  --crate firmware-esp32c6-devkitm1-n4-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c6-devkitm1-n4/onboarding-smoke.yaml \
  --system configs/systems/esp32c6-devkitm1-n4.yaml \
  --out-dir examples/esp32c6-devkitm1-n4/evidence
```

The run asserts the UART marker, at least 48 GPIO8 edges, and a decoded red
pixel. See [validation](VALIDATION.md), [known limits](KNOWN_LIMITATIONS.md),
and the [vendor availability record](evidence/vendor-status.md).
