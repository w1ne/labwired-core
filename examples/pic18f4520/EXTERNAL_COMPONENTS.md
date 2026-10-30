# External Components — PIC18F4520

| Component | Connection | Modelled |
| :--- | :--- | :--- |
| 16 MHz crystal | OSC1/OSC2 (HS) | No — `cpu_hz` comes from the descriptor/manifest |
| USB-serial adapter or RS-232 driver | RC6/TX, RC7/RX | TX only — console capture |
| LED + resistor | RB0 | Latch state only (`LATB0`) |
