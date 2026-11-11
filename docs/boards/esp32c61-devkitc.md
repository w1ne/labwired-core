# ESP32-C61-DevKitC-1-N8R2

The ESP32-C61-DevKitC-1-N8R2 uses Espressif's ESP32-C61-WROOM-1 module.
ESP32 is the highest measured demand family in the target list: rank 1 at 38%.

## Current model coverage

| Item | Coverage |
|---|---|
| Chip descriptor | [`configs/chips/esp32c61.yaml`](../../configs/chips/esp32c61.yaml) |
| Board system | [`configs/systems/esp32c61-devkitc-1-n8r2.yaml`](../../configs/systems/esp32c61-devkitc-1-n8r2.yaml) |
| Firmware | [`crates/firmware-esp32c61-smoke/`](../../crates/firmware-esp32c61-smoke/) |
| Copy-paste example | [`examples/esp32c61-devkitc-1-n8r2/`](../../examples/esp32c61-devkitc-1-n8r2/) |
| Smoke evidence | [`examples/esp32c61-devkitc-1-n8r2/evidence/`](../../examples/esp32c61-devkitc-1-n8r2/evidence/) |

The smoke boots a bare-metal RV32IMC image on the C61's RV32IMAC core, enables
UART0's PCR clock, emits a UART marker, and toggles GPIO4 through the C61 GPIO
register map. The script asserts the UART marker, at least eight modeled GPIO4
edges, and the bounded stop condition.

## Board details and limits

Espressif's [board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c61/esp32-c61-devkitc-1/index.html)
maps GPIO4 to J1 pin 3, UART0 TX to GPIO11 (J3-2), and UART0 RX to GPIO10
(J3-3). The profile models the UART register head and PCR clock gate plus a
direct GPIO0..25 output subset. It does not model GPIO input, GPIO interrupts,
other peripheral matrix routes, GPIO26..29, IO_MUX pad routing, or electrical
levels on the headers. The GPIO4 edge assertion proves the modeled output latch
and enable path, not a package-pin voltage. Wi-Fi, Bluetooth LE, native USB,
PSRAM, and the RGB LED are not declared as supported.

See the example's [availability record](../../examples/esp32c61-devkitc-1-n8r2/vendor-status.md)
and [validation notes](../../examples/esp32c61-devkitc-1-n8r2/VALIDATION.md).
