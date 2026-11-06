# ESP32-C3-DevKit-RUST-2

**Demand rank:** ESP32 is rank 1 in the JetBrains State of Embedded survey
(38% reported use). This is a new board in the highest-demand MCU family; the
survey does not rank this exact SKU. Live vendor and distributor availability
is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

The onboarding smoke runs a Rust/RISC-V ELF through the local simulator and
checks the built-in WS2812B RGB LED on GPIO2. The profile includes the ESP32-C3
GPIO and IO_MUX register models and a one-pixel NeoPixel protocol model. The
firmware bit-bangs a 24-bit GRB red frame. The onboard ICM-42670-P IMU and
SHTC3 temperature/humidity sensor, button, plain user LED, radio, USB, and
power circuitry are outside this profile.

## Copy-paste Arduino example

Install the Adafruit NeoPixel library and upload this sketch to the board:

```cpp
#include <Adafruit_NeoPixel.h>

Adafruit_NeoPixel led(1, 2, NEO_GRB + NEO_KHZ800);

void setup() {
  led.begin();
  led.setPixelColor(0, led.Color(255, 0, 0));
  led.show();
}

void loop() {}
```

The sketch demonstrates the same GPIO2 data path exercised by the simulator
smoke. It does not claim that the simulator implements the Arduino runtime.

## Run the simulator smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32c3-devkit-rust-2 \
  --crate firmware-esp32c3-rust2-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c3-devkit-rust-2/onboarding-smoke.yaml \
  --system configs/systems/esp32c3-devkit-rust-2.yaml \
  --out-dir examples/esp32c3-devkit-rust-2/evidence
```

The smoke requires at least 48 GPIO2 edges and a decoded red pixel (`RGB565
0xF800`), then stops at the configured instruction limit. Committed logs and
metrics are under [`evidence/`](evidence/). This is repository simulator
evidence, not a physical-board test. The profile's scope and omitted hardware
are listed in [`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
