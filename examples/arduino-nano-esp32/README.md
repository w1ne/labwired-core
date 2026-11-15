# Arduino Nano ESP32

**Demand rank:** ESP32 family, rank 1 (38% of respondents in the measured
embedded survey); Arduino kits ranked third among kit categories (37%).
This adds a buyable Arduino-form-factor board over the existing ESP32-S3 model.

The current Arduino store item is the Nano ESP32, SKU ABX00092. The official
board documentation identifies the NORA-W106 module with an ESP32-S3. The
board profile maps the built-in D13/status LED to GPIO48 and selects the native
USB-Serial-JTAG console. Pin map and status evidence are in
[`vendor-status.md`](vendor-status.md).

## Model-verified example

The copy-paste sketch drives the board's D13 LED and prints through the native
USB console:

```cpp
#include <Arduino.h>
#include "driver/gpio.h"

void setup() {
  Serial.begin(115200);
  gpio_set_direction(GPIO_NUM_48, GPIO_MODE_OUTPUT);  // D13 / LED_BUILTIN.
  gpio_set_level(GPIO_NUM_48, 1);
  Serial.println("NANO_ESP32_GPIO48_OK");
}

void loop() { delay(100); }
```

`examples/arduino-nano-esp32/src/main.cpp` contains this Arduino path plus a
small fast-boot entry for the local CLI smoke. The hosted verifier passes both
the serial marker and the GPIO48 high-state oracle. Its generic board alias
`D13` is not defined in the current hosted compiler profile, and its
`LED_BUILTIN` did not drive GPIO48. The verified sketch uses ESP-IDF's native
GPIO API from within Arduino so it names the SoC pin directly and is independent
of the Nano core's selectable Arduino-pin/GPIO-number remapping.

## Local smoke

Run the model-backed GPIO48 pad smoke with:

```sh
scripts/onboarding_smoke.sh \
  --target-id arduino-nano-esp32-gpio \
  --script examples/arduino-nano-esp32/io-smoke.yaml \
  --system configs/systems/arduino-nano-esp32.yaml \
  --out-dir examples/arduino-nano-esp32/evidence/run \
  --firmware-build-script examples/arduino-nano-esp32/build-firmware.sh
```

The committed local fixture is built with PlatformIO, then its ELF entry point
is patched to a small direct-register `labwired_entry`. This bypasses Arduino
startup locally and checks two GPIO48 pad edges, from low to high. The full
Arduino sketch above was separately model-verified on the hosted twin, which
booted the application and observed its serial output and GPIO state. Logs and
structured result are under `evidence/run/`.

## Scope

This profile covers S3 GPIO, the GPIO48 D13 status LED, and native
USB-Serial-JTAG. It does not claim Wi-Fi/Bluetooth, USB-OTG, PSRAM, power
behavior, or the separate RGB LED on GPIO46/GPIO0/GPIO45. The hosted run
reported the engine's existing approximation that assumes an ideal supply
rail; it does not simulate power-on or brown-out reset. No physical board was
tested.
