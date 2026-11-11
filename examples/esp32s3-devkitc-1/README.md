# ESP32-S3-DevKitC-1 v1.1

This board profile targets the Espressif ESP32-S3-DevKitC-1 v1.1 with the
ESP32-S3-WROOM-1-N16R8 module. The supported slice is the onboard USB-to-UART
bridge and the ESP32-S3 UART0 path.

## Pin map

| Board connection | MCU signal | LabWired connection |
| --- | --- | --- |
| J3 pin 2, TX | GPIO43 / U0TXD | UART0 → `host_console` |
| J3 pin 3, RX | GPIO44 / U0RXD | UART0 ← `host_console` |

The official [ESP32-S3-DevKitC-1 v1.1 guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
lists the remaining header pin functions and the board schematic.

## Smoke and limits

Run `io-smoke.yaml` to boot the committed ESP32-S3 Tier-1 firmware and assert
its UART0 transcript. The board profile does not currently represent the
addressable RGB LED on GPIO38, PSRAM, USB-OTG, Wi-Fi/Bluetooth, or the boot and
reset buttons. It therefore demonstrates firmware boot and UART0 behavior,
not full board coverage. The USB-to-UART bridge is abstracted by the
`host_console` endpoint; the bridge IC itself is not a separate model.

Live vendor status and the selected module variant are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md). Smoke output is
committed under [`evidence/run/`](evidence/run/).
