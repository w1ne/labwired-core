# External Components — dsPIC33FJ32MC204 (M0)

None. UART1 is modelled **on the CPU** (writes to `U1TXREG` reach the console
sink directly), matching the AVR core's pattern. No bus peripherals or external
devices are attached in this example.

When peripheral milestones land (SPI/I2C kits, displays, sensors), each device
goes here with its fidelity tier and descriptor path.
