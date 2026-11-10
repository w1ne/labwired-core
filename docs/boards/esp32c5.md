# ESP32-C5-DevKitC-1-N8R8

The **ESP32-C5-DevKitC-1-N8R8 v1.2** is an Espressif development board based
on the ESP32-C5-WROOM-1 module, with 8 MB flash and 8 MB PSRAM. ESP32 is demand
rank **#1** in the JetBrains State of Embedded 2025 MCU-use survey (38%). The
survey does not separate C5 from the wider ESP32 family; this is a family-depth
pick for Espressif's newly available dual-band Wi-Fi 6 RISC-V device.

This page documents a **narrow, simulator-only UART0 + GPIO smoke**, not
complete C5 support. Firmware is loaded directly into the HP SRAM map, sends a
UART0 marker, and toggles header GPIO5. The smoke does not run Espressif's mask
ROM or boot firmware from flash.

## Status

| Aspect | Status |
|---|---|
| Chip descriptor | [`configs/chips/esp32c5.yaml`](../../configs/chips/esp32c5.yaml) |
| System | [`configs/systems/esp32c5-devkitc-1-n8r8-gpio.yaml`](../../configs/systems/esp32c5-devkitc-1-n8r8-gpio.yaml) |
| Example and evidence | [`examples/esp32c5-devkitc-1-gpio-smoke/`](../../examples/esp32c5-devkitc-1-gpio-smoke/README.md) |
| Exercised path | RISC-V base IMAC execution, UART0 FIFO/console output, and GPIO5 output edges |
| Hardware parity | Not run |

## Pin map for this smoke

| Board signal | MCU pin | Use |
|---|---|---|
| J3 pin 9 | GPIO5 | Output toggled by smoke; attach an external LED and resistor for a bench check |
| J3 pin 7 | GPIO27 | Onboard addressable RGB LED; not modeled in this smoke |
| J3 pin 2 / 3 | U0TXD / U0RXD, GPIO11 / GPIO12 | USB-to-UART bridge; UART0 TX is modeled, but pin routing is not enforced |

The C5 GPIO output model covers the direct register path for GPIO0..25. The
IOMUX words are represented as declarative register storage, so the simulator
does not enforce pad routing. The smoke observes GPIO model edges and does not
claim an electrical connection at the package pin.

## Explicit limits

The descriptor does not claim ESP32-C5 mask-ROM boot, flash/XIP, PSRAM, the LP
core, CLIC or C5-specific CSRs, Wi-Fi, Bluetooth LE, Zigbee, Thread, USB,
UART0 pin routing or PCR clock gating, GPIO26..28 matrix behavior, or the
GPIO27 addressable LED. These paths require separate models and acceptance
smokes.

The current buyability check and the exact board variant are recorded in
[`vendor-status.md`](../../examples/esp32c5-devkitc-1-gpio-smoke/vendor-status.md).
