# External Components — SAM4S Xplained

None. The smoke firmware exercises the SoC surface only: CPU, memory, PMC,
PIO and UART0. Zephyr's board documentation identifies UART0 PA9/PA10 as the
onboard SEGGER J-Link OB VCP. The simulator captures the UART stream directly;
it does not model that USB bridge or establish physical acceptance.

The board carries a touch/joystick area and an EDBG debugger; neither is
modelled or attached in this example:

| Component | Board wiring | State |
| :--- | :--- | :--- |
| SEGGER J-Link OB | SWD/debug and USB VCP | physical UART0 bridge is documented; the USB bridge itself is not simulated |
| UART0 console | PA9/PA10 | simulator output capture; physical VCP wiring follows the Zephyr board docs |
| User LED / button | PC10 / PA5 | modelled PIO pads with board_io bindings |
| Touch/joystick | PIO/ADC | not modelled |

When any of these becomes a milestone, it goes here with its fidelity tier and
descriptor path, and the system manifest grows the matching
`external_devices` entry.
