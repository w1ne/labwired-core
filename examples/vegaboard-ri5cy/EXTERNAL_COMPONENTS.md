# External Components — Vega board (RI5CY)

None. The smoke firmware exercises the SoC's own surface only: CPU, memory and
the LPUART0 console. The UART is the modelled console and needs no external
device — the engine captures its byte stream directly.

The Vega board carries sensors, radios and user I/O on real hardware; none of
it is modelled or attached in this example:

| Component | Board wiring | State |
| :--- | :--- | :--- |
| Host serial bridge | host serial ↔ the SoC's LPUART0 pins | not modelled — the console sink stands in for the bridge |
| On-board sensors / radio | I²C/SPI peripherals | not modelled |
| LEDs / buttons | GPIO | not modelled (the platform defines no GPIO bank) |

When any of these becomes a milestone, it goes here with its fidelity tier and
descriptor path, and the system manifest grows the matching
`external_devices` entry.
