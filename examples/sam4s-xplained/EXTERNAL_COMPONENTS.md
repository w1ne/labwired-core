# External Components — SAM4S Xplained

None. The smoke firmware exercises the SoC surface only: CPU, memory, PMC,
PIO and UART0. The UART is the modelled console and needs no external device —
the engine captures its byte stream directly.

The board carries a touch/joystick area and an EDBG debugger; neither is
modelled or attached in this example:

| Component | Board wiring | State |
| :--- | :--- | :--- |
| EDBG virtual COM port | UART0 PA9/PA10 | modelled as the console sink (no host bridge modelled) |
| User LED / button | PC10 / PA5 | modelled PIO pads with board_io bindings |
| Touch/joystick | PIO/ADC | not modelled |

When any of these becomes a milestone, it goes here with its fidelity tier and
descriptor path, and the system manifest grows the matching
`external_devices` entry.
