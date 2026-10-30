# External Components — EK-RA8M1

None for the UART+LED L1 slice. The smoke exercises the MCU's own SYSTEM +
MSTP + PORT/PFS + SCI9 surface; SCI9 is the modelled console and the J-Link OB
USB bridge is not modelled.

The EK-RA8M1 carries further parts, for example the Ethernet PHY, the OSPI
flash, the camera and display connectors, the microphone and the USB
connectors. **None of these are modelled or attached** in this example. The
system manifest lists only the three LEDs and the two switches as `board_io`.

When a later slice attaches one of them, it goes here with its fidelity tier
and descriptor path, and the system manifest grows an `external_devices:`
entry.
