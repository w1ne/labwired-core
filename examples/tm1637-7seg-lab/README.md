# TM1637 4-digit 7-segment display lab

Wires a RobotDyn-style TM1637 4-digit clock display to two STM32F103 GPIO lines,
`CLK` (`PA8`) and `DIO` (`PA9`). There is no hardware bus — the firmware
bit-bangs the TM1637's I²C-like, LSB-first, address-less protocol. The simulator
observes both output pins through the GPIO write-hook, decodes the
start/stop/data/ACK framing and the data/address/display-control commands, and
renders the four grids (including the clock colon) via the standard segment font.

The same descriptor can attach to RP2040 `GP2` (CLK) and `GP3` (DIO). Its SIO
path supports enabled output-latch writes and output-enable bit banging: a low
latch with OE set drives low; clearing OE releases the line to the declared
module pull-up. The model counts the ninth ACK clocks but does not drive DIO
low for the chip's ACK, so firmware that checks the electrical ACK is outside
this model's scope.

Inspection includes the last 64 bus transactions and the last 64 panel frames,
plus cumulative ACK-clock and framing-error counts. A repeated four-digit write
keeps a new frame record even when the displayed pixels are unchanged.
