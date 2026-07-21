# 74HC595 4-digit 7-segment display lab

Wires the common blue "4-bit LED display module" — two chained `74HC595` shift
registers driving a 4-digit 7-segment display — to the STM32F103 `spi1` bus.

`DIO`→MOSI, `SCLK`→SCK, `RCLK`→chip-select (`PA4`, the latch). For each digit
the firmware shifts a segment byte and a digit-select byte (16 bits) then pulses
RCLK; the model decodes the standard a–g/dp font and multiplexes the four
digits into a readable 4-character value.
