# External components — NUCLEO-F103RB

The base Nucleo smoke uses no external simulated components. It exercises the
on-chip GPIOA and USART2 paths, with board-level connections to LD2 and the
ST-LINK virtual COM port. B1 is described in `board_io` but that smoke does not
read it. The companion `nucleo-f103rb-bme280` profile attaches a BME280 at
I2C1 address `0x77`; its separate smoke reads the chip-ID register through the
modeled controller.

`external_devices: []` in the base system manifest is intentional. The sensor
profile and its PB6/PB7 wiring are documented separately in
[`../nucleo-f103rb-bme280/README.md`](../nucleo-f103rb-bme280/README.md).
