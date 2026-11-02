# External Components — SK-AM62

None. The Zephyr images use the SoC's own MAIN_UART0 console and the A53
generic timer; the kit's FTDI USB-UART bridge, LEDs, user button, I2C GPIO
expanders, Ethernet PHYs and expansion headers are not modelled or attached.
When a later slice attaches one, it goes here with its fidelity tier and
descriptor path.
