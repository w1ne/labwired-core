# ESP32-C3-DevKitM-1-N4X

**Demand rank:** ESP32 is rank 1 in the 2025 JetBrains State of Embedded MCU
survey (38%). This round adds a smoke for Espressif's exact N4X DevKitM-1
board SKU. The vendor guide marks the older unsuffixed board EOL; this N4X
variant is listed as Active and in stock. See
[`evidence/vendor-status.md`](evidence/vendor-status.md).

The narrow profile loads the ESP32-C3 GPIO and IO_MUX models and attaches a
protocol-level NeoPixel model to the board's built-in GPIO8 RGB LED (J3 pin 6,
schematic reference D6, SK68XXMINI-HS). Firmware sends a red 24-bit GRB frame;
the smoke checks the GPIO waveform and decoded red pixel. GPIO8 is also a boot
strapping pin. Radio, USB, the USB-to-UART bridge, the boot button, and other
peripherals are outside this profile.

## Copy-paste Arduino example

Install the Adafruit NeoPixel library, then upload this sketch to the board:

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

GPIO8 (J3 pin 6) is connected to the on-board addressable RGB LED in the
current ESP32-C3-DevKitM-1 guide. The protocol smoke checks the one-wire data
path; it does not model LED electrical behavior or the exact LED die.

## Run the simulator smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32c3-devkitm1-n4x \
  --crate firmware-esp32c3-devkitm1-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c3-devkitm1-n4x/onboarding-smoke.yaml \
  --system configs/systems/esp32c3-devkitm1-n4x.yaml \
  --out-dir examples/esp32c3-devkitm1-n4x/evidence
```

The committed evidence directory contains the firmware build log, simulator
smoke log, and the generated onboarding metrics. This is model evidence, not a
physical-board test. The hosted verifier limitation is recorded in
[`evidence/labwired-verification.md`](evidence/labwired-verification.md).
