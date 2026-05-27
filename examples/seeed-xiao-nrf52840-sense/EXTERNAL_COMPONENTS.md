# External Components

No external SPI, I2C, or analog components are required for the current smoke
test. The target models the on-board RGB LED and the MCU SPIM0 controller
registers only.

Future SPI device validation should add an explicit external device manifest
entry plus a hardware trace captured with SWD or a logic analyzer.
