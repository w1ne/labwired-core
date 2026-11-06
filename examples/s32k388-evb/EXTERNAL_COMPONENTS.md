# External Components — S32K388EVB-Q289

None for the LPUART+LED L1 slice. The smoke exercises the MCU's own SWT_0 +
MC_ME + SIUL2 + LPUART2 surface; LPUART2 is the modelled console and the
EVB's USB-UART bridge is not modelled.

The S32K388EVB-Q289 carries further parts, for example the FS26 safety
system basis chip, the CAN FD / LIN transceivers, the Ethernet PHY and the
ADC rotary potentiometers. **None are modeled as board components or
controls** in this example. The analog co-simulation fixture injects generic
NTC and diode voltages at the potentiometer input pads; it does not emulate
the potentiometers themselves. The system manifest lists only the green LED
(PTF23) as `board_io`.

When a later slice attaches one of them, it goes here with its fidelity tier
and descriptor path, and the system manifest grows an `external_devices:`
entry.
