# L4 negative control: no MAX31855 on SPI0

Same `firmware.elf` as the passing L4 cell. The only change is the board
(`arduino-nano-every-no-max31855.yaml`): no device is attached to SPI0.

Result: FAIL. The sketch prints `LW_L4_FAIL frame=0xffffffff` (`uart.log`):
no selected device drives MISO, so each of the four `SPI.transfer()` calls
receives 0xFF. The passing cell's `LW_L4_OK` therefore needs the device on
the bus and its chip select on SS (D8, PE3). That cell is the exact tier:
the frame equals `0x01901600`.
