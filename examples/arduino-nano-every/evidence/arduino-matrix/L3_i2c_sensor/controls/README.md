# L3 negative control: no INA219 on TWI0

Same `firmware.elf` as the passing L3 cell. The only change is the board
(`arduino-nano-every-no-ina219.yaml`): no device is attached to TWI0.

Result: FAIL. The sketch prints `LW_L3_FAIL nack` (`uart.log`): the address
byte gets no acknowledge, TWI0 sets WIF with RXACK, and stock
`Wire.endTransmission()` returns a non-zero code. The passing cell's
`LW_L3_OK` therefore needs the device on the bus. That cell is the exact
tier: both INA219 register reads (`0x399F`, `0x19CA`) match, and the sketch
prints no `LW_L3_PARTIAL_NO_RX` line.
