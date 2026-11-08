# Waveshare ESP32-C3-Zero

**Demand rank:** ESP32 is rank **#1** in the measured MCU usage list (38%). This
round adds the exact Waveshare ESP32-C3-Zero board, SKU 25452, with a smoke for
its onboard WS2812 RGB LED. Waveshare's live store lists it at $3.49–$4.49 with
an Add to Cart option; see [`evidence/vendor-status.md`](evidence/vendor-status.md).

The focused board profile models the ESP32-C3 memory map, GPIO output, GPIO10
IO_MUX window, and the LED's one-wire pixel protocol. The smoke sends a red
24-bit GRB frame on GPIO10 and checks both the GPIO waveform and the decoded
pixel. The RMT controller, radio, USB, UART, buttons, power circuitry, and other
header pins are outside this profile.

## Copy-paste Arduino example

Install the Adafruit NeoPixel library, select **ESP32C3 Dev Module**, and enable
USB CDC on boot if you want the Arduino console. Upload:

```cpp
#include <Adafruit_NeoPixel.h>

Adafruit_NeoPixel led(1, 10, NEO_GRB + NEO_KHZ800);

void setup() {
  led.begin();
  led.setPixelColor(0, led.Color(255, 0, 0));
  led.show();
}

void loop() {}
```

GPIO10 is connected to the onboard WS2812 RGB LED on both the no-header
ESP32-C3-Zero and headered ESP32-C3-Zero-M variants.

This sketch is a copy-paste example for the physical board. The repository
smoke sends the same pixel frame with direct GPIO writes; it does not assert
the behavior of the Arduino core or NeoPixel library's waveform-generation
path.

## Run the simulator smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id waveshare-esp32c3-zero \
  --crate firmware-esp32c3-waveshare-c3-zero-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/waveshare-esp32c3-zero/onboarding-smoke.yaml \
  --system configs/systems/waveshare-esp32c3-zero.yaml \
  --out-dir examples/waveshare-esp32c3-zero/evidence
```

The committed evidence includes build logs, the local simulator smoke output,
and its generated onboarding metrics. This is model evidence, not a physical
board test.
