# External Components — Curiosity PIC32MZ EF 2.0

None. The NuttX images use the MCU's own UART2 console and Timer1; the
board's PKoB4 debugger (only its virtual COM port role is modelled, as the
console), LEDs, switches, mikroBUS and Arduino headers, the Ethernet PHY
and the SQI flash are not modelled or attached. When a
later slice attaches one, it goes here with its fidelity tier and
descriptor path.
