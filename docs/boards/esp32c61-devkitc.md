# ESP32-C61-DevKitC-1-N8R2

The ESP32-C61-DevKitC-1-N8R2 is an Espressif board built around the
ESP32-C61-WROOM-1 module. ESP32 is the highest measured demand family in the
current onboarding target list (38% reported usage).

## Current model coverage

| Item | Coverage |
|---|---|
| Chip descriptor | [`configs/chips/esp32c61.yaml`](../../configs/chips/esp32c61.yaml) |
| Board system | [`configs/systems/esp32c61-devkitc-1-n8r2.yaml`](../../configs/systems/esp32c61-devkitc-1-n8r2.yaml) |
| Firmware | [`crates/firmware-esp32c61-smoke/`](../../crates/firmware-esp32c61-smoke/) |
| Copy-paste example | [`examples/esp32c61-devkitc-1-n8r2/`](../../examples/esp32c61-devkitc-1-n8r2/) |
| Smoke evidence | [`examples/esp32c61-devkitc-1-n8r2/evidence/`](../../examples/esp32c61-devkitc-1-n8r2/evidence/) |

The smoke boots a bare-metal RV32IMAC image, enables UART0's PCR clock, sets the
UART divider, and emits `ESP32C61_DEVKITC1_UART0_OK`. The captured FIFO output
and bounded stop are asserted by the smoke script.

## Board details and limits

Espressif's [board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c61/esp32-c61-devkitc-1/index.html)
documents UART0 TX on GPIO11 (J3-2) and UART0 RX on GPIO10 (J3-3), with the
board's USB-UART bridge connected to that console. The current profile models
the UART register head and PCR clock gate only. It does not model GPIO/IO_MUX
pad routing or the electrical USB-UART bridge path; captured UART bytes are a
peripheral-level smoke. Other peripherals, including radios, native USB, and
the RGB LED, are not declared as supported.

See the example's [availability record](../../examples/esp32c61-devkitc-1-n8r2/vendor-status.md)
and [validation notes](../../examples/esp32c61-devkitc-1-n8r2/VALIDATION.md).
