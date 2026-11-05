# ESP32-DevKitC-32E GPIO23 smoke

**Demand rank:** ESP32 is rank 1 in the demand list (38% in the 2025 JetBrains
State of Embedded MCU survey). This adds a board-level profile for the common
ESP32-DevKitC-32E using the existing classic ESP32 model; it does not add a new
chip core.

## Board and scope

The board is Espressif's ESP32-DevKitC-32E, carrying an ESP32-WROOM-32E with
4 MB flash. The narrow profile exposes the real GPIO model and maps J3 pin 2 to
GPIO23. The physical board has a 5 V power indicator rather than a GPIO-driven
user LED, so the smoke observes GPIO23 directly and does not invent an
on-board LED. To see a blink on physical hardware, connect an LED in series
with a resistor from GPIO23 to GND.

| Board connection | MCU signal | Smoke |
| --- | --- | --- |
| J3 pin 2 | GPIO23 | Arduino firmware toggles output; local assertion requires 16 edges |

This profile does not claim UART0 or the USB-to-UART bridge, Wi-Fi/Bluetooth,
I2C, SPI, timers, flash boot behavior, or the power LED. Espressif's live guide
and the current lifecycle/availability check are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Firmware

The committed sketch uses the ESP-IDF GPIO API exposed by Arduino. It is the
same simple example you can copy into `src/main.cpp` in a PlatformIO
`esp32dev` Arduino project:

```cpp
#include <Arduino.h>
#include <driver/gpio.h>

constexpr gpio_num_t kHeaderPin = GPIO_NUM_23;

void setup() {
  gpio_reset_pin(kHeaderPin);
  gpio_set_direction(kHeaderPin, GPIO_MODE_OUTPUT);
  gpio_set_level(kHeaderPin, 0);
}

void loop() {
  static bool output_high = false;
  output_high = !output_high;
  gpio_set_level(kHeaderPin, output_high ? 1 : 0);
}
```

## Reproduce

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-devkitc-32e \
  --script examples/esp32-devkitc-32e/io-smoke.yaml \
  --system configs/systems/esp32-devkitc-32e-gpio-smoke.yaml \
  --out-dir examples/esp32-devkitc-32e/evidence \
  --firmware-build-script scripts/build-esp32-devkitc-32e-smoke.sh
```

The firmware build log, local simulator result, and generated onboarding
metrics are committed under `evidence/`. The local CLI run passed after
660,000 steps with 479 captured GPIO23 transitions. This is digital-twin smoke
evidence, not a physical-board run.

The separate hosted LabWired verifier observed the GPIO toggle but returned
`unsupported`: the classic ESP32 hosted profile then faulted on an unmapped
read at `0x3fc9b2b4` (faulting PC `0x4008813e`). The full result is preserved
in [`evidence/hosted-verification.json`](evidence/hosted-verification.json);
it is not a model-verification pass. The narrow repository profile uses the
native ESP32 GPIO model and no zero-answering peripheral stubs.
