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

## Stock Arduino matrix row (TinyUSB CDC on USB-OTG)

The section above runs a patched fast entry and the USB-Serial-JTAG console.
Stock Arduino code on this board does not use that console. The board JSON
leaves `ARDUINO_USB_MODE` unset, so `Serial` is TinyUSB CDC on the USB-OTG
controller. It drops writes until a host opens the port (DTR). The JSON also
sets `BOARD_HAS_PIN_REMAP`, so sketches use Arduino pin numbers (D13 =
GPIO48, D2 = GPIO5, A4/A5 = GPIO11/12).

The matrix row builds the nine unmodified-core sketches with PlatformIO and
boots each through the full Arduino startup on both cores. The twin's
scripted USB host enumerates the device, sends SET_CONFIGURATION, opens the
first CDC-ACM port and reads its bulk IN data. L0–L2 print only on `Serial`,
so their markers arrive over USB CDC. L3–L8 also mirror each line to
`Serial0` (UART0).

```sh
python3 validation/arduino-matrix/run_matrix.py --boards arduino_nano_esp32
```

Result on 2026-11-16: 9 pass / 0 skip / 0 fail. L2 sees 3 edges on GPIO48
(`LED_BUILTIN`); L6 sees 59 LEDC edges on GPIO5. Each cell costs about 213M
steps, because the stock variant runs `boot_double_tap_logic()` (an RGB fade
of about 1 s) before `setup()`. Per-cell files are in
`evidence/arduino-matrix/`.

The matrix row uses its own system file
(`validation/arduino-matrix/systems/arduino_nano_esp32.yaml`, with the L3–L5
sensors). This board profile is unchanged. PSRAM is not modelled: the probe
reports "PSRAM chip is not connected" and the firmware continues.

## Scope

This profile covers S3 GPIO, the GPIO48 D13 status LED, and native
USB-Serial-JTAG. It does not claim Wi-Fi/Bluetooth, USB-OTG, PSRAM, power
behavior, or the separate RGB LED on GPIO46/GPIO0/GPIO45. The hosted run
reported the engine's existing approximation that assumes an ideal supply
rail; it does not simulate power-on or brown-out reset. No physical board was
tested.

## External I²C sensor example

The matching [BME280 example](../arduino-nano-esp32-bme280/README.md) uses the
board's A4/SDA and A5/SCL pins (GPIO11/GPIO12) and checks the sensor's 0x60 chip
ID through the modelled I²C0 path.
