# External Components — CC2538DK

None for the UART+LED L1 slice. The smoke exercises the MCU's own SYS_CTRL +
GPIO + UART0 surface; UART0 is the modelled console and the SmartRF06 EB's USB
debugger bridge is not modelled.

The SmartRF06 EB carries further parts — for example the ambient-light
sensor that Contiki-NG `dev/board.h` wires to PA6 (output) / PA7 (power).
**None of these are modelled or attached** in this example — the system manifest lists only the four LEDs
and the five keys as `board_io`.

When a later slice attaches one of them, it goes here with its fidelity tier
and descriptor path, and the system manifest grows an `external_devices:`
entry.
