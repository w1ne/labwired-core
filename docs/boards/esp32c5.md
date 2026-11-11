# ESP32-C5-DevKitC-1-N8R8

The **ESP32-C5-DevKitC-1-N8R8 v1.2** is based on ESP32-C5-WROOM-1 with 8 MB
flash and 8 MB PSRAM. ESP32 is demand rank **#1** in the JetBrains State of
Embedded 2025 MCU-use survey (38%). The survey does not separate C5 from the
wider ESP32 family, so this is a family-depth pick, not a claim that the model
covers every C5 block.

This focused system boots a bare-metal RV32IMAC image into HP SRAM, reads a
BME280's chip ID and raw temperature through C5 I2C0 on GPIO8/9, reports a
success marker through UART0, and toggles GPIO5. It is local simulator-smoke evidence;
it does not boot the mask ROM or flash image.

## Status

| Aspect | Status |
|---|---|
| Chip descriptor | [`configs/chips/esp32c5.yaml`](../../configs/chips/esp32c5.yaml) |
| System | [`configs/systems/esp32c5-devkitc-1-n8r8-bme280.yaml`](../../configs/systems/esp32c5-devkitc-1-n8r8-bme280.yaml) |
| Example and evidence | [`examples/esp32c5-devkitc-1-bme280-smoke/`](../../examples/esp32c5-devkitc-1-bme280-smoke/README.md) |
| Exercised path | HP RV32IMAC, modeled UART0 output, GPIO5 edges, I2C0 command list, C5 GPIO matrix GPIO8/9, and BME280 ID/raw-temperature reads |
| Hardware parity | Not run |

## Pin map

| Board header | MCU pin | Use |
|---|---|---|
| J1 pin 7 | GPIO6 | Available but unused by this smoke |
| J1 pin 9 | GPIO8 | BME280 SDA |
| J1 pin 10 | GPIO9 | BME280 SCL |
| J3 pin 2 | GPIO11 / U0TXD | UART0 console signal in the board guide; simulator UART pin routing is not enforced |
| J3 pin 9 | GPIO5 | GPIO output-edge smoke |
| J3 pin 7 | GPIO27 | Onboard addressable RGB LED, not modeled |

GPIO7 is a strapping pin, so the sensor example avoids it. GPIO8/9 are exposed
at the J1 headers. The C5 GPIO model covers direct registers for GPIO0..25 and
the C5 input/output matrix registers needed for I2C0 signals 46/47. Other C5
matrix blocks and GPIO26..28 are outside this model.

The IO_MUX window is declarative register storage; pad electrical routing and
UART0's route to the USB-to-UART bridge are not enforced. GPIO5 assertions
observe model edges rather than package voltage. The C5 PCR clock gate is also
not enforced.

## Explicit limits

This descriptor does not claim mask-ROM boot, flash/XIP, PSRAM, the LP core,
CLIC or C5-specific CSRs, Wi-Fi, Bluetooth LE, Zigbee, Thread, USB, full GPIO
matrix behavior, or the GPIO27 addressable LED. These paths need their own
models and smoke evidence. No zero-answering peripheral stub is used by the
smoke; the IO_MUX remains declarative and other unsupported systems are simply
outside the descriptor.

The current vendor availability check is in
[`vendor-status.md`](../../examples/esp32c5-devkitc-1-bme280-smoke/vendor-status.md).
