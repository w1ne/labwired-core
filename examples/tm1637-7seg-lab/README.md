# TM1637 4-digit 7-segment display lab

Wires a RobotDyn-style TM1637 4-digit clock display to two STM32F103 GPIO lines,
`CLK` (`PA8`) and `DIO` (`PA9`). There is no hardware bus — the firmware
bit-bangs the TM1637's I²C-like, LSB-first, address-less protocol. The simulator
observes both output pins through the GPIO write-hook, decodes the
start/stop/data/ACK framing and the data/address/display-control commands, and
renders the four grids (including the clock colon) via the standard segment font.
