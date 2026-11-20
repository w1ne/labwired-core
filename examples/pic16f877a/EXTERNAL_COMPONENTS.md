# External Components — PIC16F877A

| Component | Connection | Modelled |
| :--- | :--- | :--- |
| 16 MHz crystal | OSC1/OSC2 (HS) | No — `cpu_hz` comes from the descriptor/manifest |
| USB-serial adapter or RS-232 driver | RC6/TX, RC7/RX | TX only — console capture |
| LED + resistor | RB0 | Latch state only (PORTB latch bit 0) |
| 10 kΩ pull-up + 10 kΩ Beta 3950 NTC to VSS | RA0/AN0 | Yes — MNA netlist in `analog-ntc-diode-system.yaml`, converted by the A/D module |
| 1 kΩ from VDD + diode to VSS | RA1/AN1 | Yes — Shockley model in the same fixture, converted by the A/D module |
