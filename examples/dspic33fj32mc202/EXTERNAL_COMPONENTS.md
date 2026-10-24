# External Components — dsPIC33FJ32MC202 (M0)

None. UART1 is modelled **on the CPU** (writes to `U1TXREG` reach the console
sink directly), matching the dsPIC33FJ32MC204 example. No bus peripherals or
external devices are attached in this M0 example.

Future milestones that use the I²C/SPI kit devices will list them here with
their fidelity tier and descriptor path.
