# ESP32-H2-DevKitM-1-N4S

The ESP32-H2 board adds Thread and Zigbee capability to the ESP32 family, the measured demand rank 1 family (38%). H2 itself has no separate survey rank.

## Model and evidence

| Item | Coverage |
|---|---|
| Chip descriptor | [configs/chips/esp32h2.yaml](../../configs/chips/esp32h2.yaml) |
| Board system | [configs/systems/esp32h2-devkitm-1-n4s.yaml](../../configs/systems/esp32h2-devkitm-1-n4s.yaml) |
| Firmware | [firmware crate](../../crates/firmware-esp32h2-devkitm-1-smoke/) |
| Copy-paste example | [example and instructions](../../examples/esp32h2-devkitm-1-n4s/) |
| Smoke evidence | [committed evidence](../../examples/esp32h2-devkitm-1-n4s/evidence/) |

The bare-metal RV32IMAC smoke writes a marker to the simulator's UART0 console and toggles GPIO10 through the direct GPIO output model. On the physical board, UART0 uses GPIO24 TX and GPIO23 RX for the USB-to-UART bridge, but this profile does not simulate that pin route or the bridge. GPIO10 is a header pin; connect an LED with a series resistor to GND to try the example on a board.

This focused profile does not claim GPIO electrical routing, flash boot/XIP, USB Serial/JTAG, BLE, Thread/Zigbee, or the addressable RGB LED on GPIO8. The GPIO edge assertion is simulator evidence for the modeled latch and enable path, not a voltage or silicon measurement. See the [availability check](../../examples/esp32h2-devkitm-1-n4s/vendor-status.md) and [validation boundary](../../examples/esp32h2-devkitm-1-n4s/VALIDATION.md).
