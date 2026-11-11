# Olimex ESP32-P4-DevKit GPIO2 LED

This focused model boots a bare RV32IMAC image from the P4's internal HP SRAM,
sets the GPIO2 IO_MUX function and output enable, and toggles the GPIO2 pad
that drives the board's active-low USER_LED1. The simulation checks for eight
GPIO2 pad edges; it does not model the LED's electrical or optical response.

ESP32 ranks **#1 by demand (38%)** in the onboarding evidence. This board is a
family-depth pick; the survey does not rank ESP32-P4 separately. The profile
does not model the second CPU, F/D/custom instructions, ROM/XIP boot, PSRAM,
USB, Ethernet, camera/display, SDMMC, or the LP core.

## Copy-paste Arduino example

Olimex lists Arduino IDE support for the board. With the ESP32-P4 board target
selected, this blinks USER_LED1:

```cpp
#include <Arduino.h>

constexpr int USER_LED = 2;

void setup() {
  pinMode(USER_LED, OUTPUT);
}

void loop() {
  digitalWrite(USER_LED, LOW);  // active-low LED on
  delay(500);
  digitalWrite(USER_LED, HIGH); // LED off
  delay(500);
}
```

## Reproduce the modeled smoke

```sh
rustup target add riscv32imac-unknown-none-elf
scripts/onboarding_smoke.sh \
  --target-id olimex-esp32-p4-devkit \
  --crate firmware-olimex-esp32-p4-devkit-smoke \
  --target riscv32imac-unknown-none-elf \
  --script examples/olimex-esp32-p4-devkit/gpio2-led-smoke.yaml \
  --system configs/systems/olimex-esp32-p4-devkit.yaml \
  --out-dir examples/olimex-esp32-p4-devkit/evidence
```

See [VALIDATION.md](VALIDATION.md) for the modeled scope and evidence files,
and [vendor-status.md](vendor-status.md) for the live purchase check.
