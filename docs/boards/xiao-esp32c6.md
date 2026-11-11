# Seeed XIAO ESP32-C6

The XIAO ESP32-C6 uses Espressif's ESP32-C6FH4 (RISC-V HP core, 160 MHz,
512 KiB SRAM, 4 MiB in-package flash). It is an ESP32-family board, demand
rank **#1** in the measured target list. This adds a board profile using the
existing ESP32-C6 silicon model.

## Board wiring in the profile

| XIAO label | GPIO | Profile use | Evidence |
|---|---:|---|---|
| D4 / SDA | 22 | BME280 I²C0 SDA | Firmware reads CHIP_ID and raw temperature |
| D5 / SCL | 23 | BME280 I²C0 SCL | Firmware reads CHIP_ID and raw temperature |
| TX / D6 | 16 | UART TX pin | Pin mapping recorded; the smoke console is native USB Serial/JTAG |
| RX / D7 | 17 | UART RX pin | Pin mapping recorded; peripheral traffic is not used in the smoke |
| User LED | 15 | Active-low GPIO indicator | GPIO transition assertion in the smoke |
| BOOT | 9 | Active-low button binding | Declared; reset/download behavior is outside the smoke |
| USB-C | — | Native USB Serial/JTAG console | Firmware marker captured by the local C6 peripheral model |

Attach an Adafruit BME280 STEMMA QT breakout to D4/SDA, D5/SCL, 3V3 and GND.
The profile uses I²C address `0x77`. Follow the board's 3.3 V electrical
limits when wiring physical hardware.

## Configuration and runnable example

- Board system: [`configs/systems/xiao-esp32c6-bme280.yaml`](../../configs/systems/xiao-esp32c6-bme280.yaml)
- ESP32-C6FH4 chip profile: [`configs/chips/esp32c6fh4.yaml`](../../configs/chips/esp32c6fh4.yaml)
- Arduino firmware and local smoke: [`examples/xiao-esp32c6-bme280/`](../../examples/xiao-esp32c6-bme280/)
- Current vendor availability: [`vendor-status.md`](../../examples/xiao-esp32c6-bme280/vendor-status.md)

The smoke exercises native USB Serial/JTAG, the modeled C6 I²C0 route, the
BME280 identity and temperature registers, and GPIO15. The engine does not
claim Wi-Fi, BLE, Thread/Zigbee, the low-power core, RF, charging, USB device
enumeration, or a physical board boot for this example. See the example's
[`VALIDATION.md`](../../examples/xiao-esp32c6-bme280/VALIDATION.md) for the
evidence boundary.

Board references: [Seeed getting-started guide](https://wiki.seeedstudio.com/xiao_esp32c6_getting_started/),
[official schematic](https://files.seeedstudio.com/wiki/SeeedStudio-XIAO-ESP32C6/XIAO_ESP32_C6_v1.0_SCH_260114.pdf),
and [ESP32-C6 datasheet](https://documentation.espressif.com/esp32-c6_datasheet_en.html).
