# External component (NUCLEO-H533RE)

The smoke uses an Adafruit BME280 I2C/SPI breakout (product 2652), connected to
I2C1 on Arduino D15/PB6 (SCL) and D14/PB7 (SDA), with address `0x77`. It reads
the modelled `CHIP_ID` register at `0xD0`. The board's LD2/PA5 and default
ST-LINK USART2 VCP route are also exercised.

The breakout provides pull-ups and level shifting for 3.3 V or 5 V logic. The
simulator does not model electrical levels, pull-up resistance, environmental
conversion timing, or physical board wiring.
