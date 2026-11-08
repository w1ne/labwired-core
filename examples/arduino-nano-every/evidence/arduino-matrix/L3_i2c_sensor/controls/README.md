# L3 negative control: INA219 at 0x41

Same `firmware.elf` as the passing L3 cell. The only change is the board
(`arduino-nano-every-ina219-0x41.yaml`): the INA219 answers at 0x41 instead
of 0x40.

Result: FAIL. The sketch prints `LW_L3_FAIL nack` (`uart.log`): the TWI0
address phase at 0x40 finds no slave, sets WIF with RXACK, and stock Wire's
`endTransmission()` returns 3. The passing cell's `LW_L3_OK` therefore comes
from the device on the bus, not from an ACK-everything controller.
