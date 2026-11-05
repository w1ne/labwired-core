# ESP32-DevKitC-32E

Espressif's ESP32-DevKitC-32E carries an ESP32-WROOM-32E. The narrow LabWired
profile exercises GPIO23 and UART0 output using Arduino firmware.

| Item | Coverage |
| --- | --- |
| System | [`configs/systems/esp32-devkitc-32e-gpio-smoke.yaml`](../../configs/systems/esp32-devkitc-32e-gpio-smoke.yaml) |
| Chip profile | [`configs/chips/esp32-devkitc-32e-gpio-smoke.yaml`](../../configs/chips/esp32-devkitc-32e-gpio-smoke.yaml) |
| Firmware and evidence | [`examples/esp32-devkitc-32e/`](../../examples/esp32-devkitc-32e/README.md) |
| Smoke assertions | UART0 marker and at least 16 GPIO23 edges |
| Demand rank | ESP32: 1 (38% in the referenced MCU survey) |

The profile includes the GPIO controller and UART0 model. It does not model
the board's USB-to-UART bridge or pin-level UART TX/RX waveforms, so the UART
console capture does not prove USB serial or board-header signal behavior.
Wi-Fi/Bluetooth, I2C, SPI, timers, and the power LED are also omitted. The
board has no GPIO user LED; an external LED and resistor on GPIO23 are needed
for a physical blink. The local CLI result and hosted verifier result are
reported separately in the example evidence; the hosted verifier returns
`unsupported` for serial on the classic-ESP32 profile.

The wider classic ESP32 bus still has zero-answering `SystemStub` windows for
Wi-Fi MAC/PHY and unused I2S/PCNT/RMT/UHCI blocks. APB_CTRL has a separate
register placeholder, and the analog-AHB catch-all reads as zero. This narrow
smoke does not claim those peripherals.

Espressif's [live DevKitC V4 guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
identifies this exact variant and lists the USB-to-UART bridge. The live
availability check is in the example evidence.
