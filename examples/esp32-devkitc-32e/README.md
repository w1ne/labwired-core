# ESP32-DevKitC-32E GPIO + UART0 console smoke

**Demand rank:** ESP32 is rank 1 in the demand list (38% in the 2025 JetBrains
State of Embedded MCU survey). This extends the existing ESP32-DevKitC-32E
profile from GPIO-only to GPIO plus UART0 output; it does not add a chip or
board variant.

## Board and scope

The board is Espressif's ESP32-DevKitC-32E with an ESP32-WROOM-32E module and
4 MB flash. The narrow profile exposes the classic ESP32 GPIO and UART0 models.
It maps J3 pin 2 to GPIO23. The smoke observes GPIO23 directly and does not
invent an on-board user LED; the board's 5 V power indicator is not GPIO-driven.
For a physical blink, connect an LED in series with a resistor from GPIO23 to
GND.

| Board connection | MCU behavior | Smoke |
| --- | --- | --- |
| J3 pin 2 | GPIO23 | Arduino firmware toggles output; local assertion requires 16 edges |
| UART0 output | Serial console capture | Sends `ESP32_DEVKITC32E_UART0_OK`; local assertion checks the UART marker |

The UART0 controller is modeled. The board's USB-to-UART bridge and the
pin-level UART TX/RX waveforms are not, so the console capture does not prove
USB serial behavior or a signal at the board header. Wi-Fi/Bluetooth, I2C,
SPI, timers, flash boot behavior, and the power LED are also outside this
profile. Espressif's live guide and the lifecycle/availability check are in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Firmware

The committed sketch uses Arduino `Serial` and the ESP-IDF GPIO API. Copy it
into `src/main.cpp` in a PlatformIO `esp32dev` Arduino project:

```cpp
#include <Arduino.h>
#include <driver/gpio.h>

constexpr gpio_num_t kHeaderPin = GPIO_NUM_23;

void setup() {
  Serial.begin(115200);
  Serial.println("ESP32_DEVKITC32E_UART0_OK");

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
metrics are committed under `evidence/`. The smoke requires the UART marker and
at least 16 GPIO23 edges. This is local digital-twin evidence, not a
physical-board run.

The hosted `labwired_verify` rejects a serial oracle for the classic ESP32 with
`SERIAL_NOT_OBSERVABLE`: that hosted profile stubs `HardwareSerial`. Its result
is recorded separately from the local CLI smoke as `unsupported`; it is not a
model-verification pass. The exercised GPIO and UART blocks use real models.
The wider classic ESP32 builder still installs zero-answering `SystemStub`
windows for Wi-Fi MAC/PHY and unused I2S/PCNT/RMT/UHCI blocks. APB_CTRL has a
separate register placeholder, and the analog-AHB catch-all reads as zero.
This smoke does not assert behavior from those blocks.
