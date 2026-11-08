# L4 negative control: MAX31855 at 100 °C

Same `firmware.elf` as the passing L4 cell. The only change is the board
(`arduino-nano-every-max31855-100c.yaml`): the kit's `temperature` input is
seeded to 100 °C instead of the 25 °C default.

Result: FAIL. The sketch prints `LW_L4_FAIL frame=0x06401600` (`uart.log`):
100 °C is 400 quarter-degrees at bit 18 (0x06400000), plus the unchanged
22 °C internal reading at bit 4 (0x1600). The passing cell's frame therefore
comes from the device through SPI0, MSB first and in order, not from a
constant.
