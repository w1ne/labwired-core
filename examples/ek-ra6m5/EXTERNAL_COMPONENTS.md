# External Components — EK-RA6M5

None for the UART+LED L1 slice. The smoke exercises the MCU's own SYSTEM +
MSTP + PORT/PFS + SCI0 surface; SCI0 is the modelled console and the J-Link OB
USB bridge is not modelled.

The EK-RA6M5 carries further parts, for example the Ethernet PHY, the OSPI and
QSPI flash, and the USB connectors. **None of these are modelled or attached**
in this example. The system manifest lists only the three LEDs and the two
switches as `board_io`.

When a later slice attaches one of them, it goes here with its fidelity tier
and descriptor path, and the system manifest grows an `external_devices:`
entry.
