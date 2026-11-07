# External Components — SAM E70 Xplained

None for the USART+LED L1 slice. The smoke exercises the MCU's own WDT +
MATRIX + PMC + PIO + USART1 surface; USART1 is the modelled console and the
EDBG USB bridge is not modelled.

The SAM E70 Xplained carries further parts, for example the KSZ8081 Ethernet
PHY, the IS42S16100E SDRAM, the SST26VF064B QSPI flash, the AT24MAC402 EEPROM
and the CAN transceiver. **None of these are modelled or attached** in this
example. The system manifest lists only LED0 and SW0 as `board_io`.

The analog co-simulation fixture injects generic NTC and diode netlist voltages
at the board's EXT1 ADC pins. It does not instantiate an external analog part,
the AFEC peripherals, or an ADC conversion path.

When a later slice attaches one of them, it goes here with its fidelity tier
and descriptor path, and the system manifest grows an `external_devices:`
entry.
