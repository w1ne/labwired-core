# Espressif ESP32-C3-DevKitM-1-N4X

**Demand rank:** ESP32 ranks first in the referenced JetBrains State of
Embedded MCU survey (38%). This board adds a current Espressif DevKit SKU to the
ESP32 family coverage.

## Product and model scope

| Item | Coverage |
|---|---|
| Board SKU | `ESP32-C3-DevKitM-1-N4X` (4 MB flash) |
| Chip/system profile | [`configs/chips/esp32c3-devkitm1-n4x-smoke.yaml`](../../configs/chips/esp32c3-devkitm1-n4x-smoke.yaml) · [`configs/systems/esp32c3-devkitm1-n4x.yaml`](../../configs/systems/esp32c3-devkitm1-n4x.yaml) |
| Exercised path | GPIO8 (J3 pin 6) → on-board addressable RGB LED (schematic ref D6, `SK68XXMINI-HS`) |
| Smoke | [Firmware example and evidence](../../examples/esp32c3-devkitm1-n4x/README.md) |
| Vendor status | [Live exact-SKU availability record](../../examples/esp32c3-devkitm1-n4x/evidence/vendor-status.md) |

Espressif's live guide lists the N4X SKU and marks the older unsuffixed
DevKitM-1 variant EOL. The exact N4X SKU was listed Active and in stock at
DigiKey when checked on 2026-11-05. The profile is deliberately narrow: only
the C3 GPIO and IO_MUX register paths needed for the RGB LED are declared. The
radio, USB ports and bridge, buttons, and other peripherals are omitted.
GPIO8 is also an ESP32-C3 boot strapping pin; avoid external circuitry that
changes its level during reset.

## GPIO8 RGB LED smoke

The firmware bit-bangs a red, 24-bit GRB frame on GPIO8. The deterministic
smoke requires at least 48 GPIO transitions and a red pixel in the decoded
NeoPixel artifact. This models the data protocol and visible color result; it
does not model the LED's electrical behavior or exact die revision. No physical
silicon test is claimed.

The hosted verifier does not currently expose the exact board, and its Arduino
entry-path contract rejected the hosted attempt. The passing evidence is the
repository's local CLI smoke at
[`examples/esp32c3-devkitm1-n4x/evidence/`](../../examples/esp32c3-devkitm1-n4x/evidence/).
