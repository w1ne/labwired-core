# External Components — Apollo4 Plus EVB

None for the UART+LED L1 slice. The smoke exercises the SoC's own PWRCTRL +
GPIO + UART0 surface; UART0 is the modelled console and the on-board SEGGER
J-Link VCOM bridge is not modelled.

The EVB carries further parts — the audio jack, the display connector and the
MSPI memory sockets. **None of these are modelled or attached** in this
example — the system manifest lists only the three LEDs and the two buttons as
`board_io`.

When a later slice attaches one of them, it goes here with its fidelity tier
and descriptor path, and the system manifest grows an `external_devices:`
entry.
