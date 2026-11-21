# External Components — PIC16F1829 on Curiosity (DM164137)

| Component | Connection | Modelled |
| :--- | :--- | :--- |
| Internal 16 MHz HFINTOSC | (on-chip, CONFIG1 FOSC = INTOSC) | No — `cpu_hz` comes from the manifest; OSCSTAT reads the oscillators ready |
| USB-serial adapter | RB7/TX, RB5/RX on the expansion header | TX only — console capture |
| LED D4 | RA5 | Latch state only (LATA bit 5) |
| 10 kΩ pull-up + 10 kΩ Beta 3950 NTC to VSS | RC0/AN4 (the POT1 net) | Yes — MNA netlist in `analog-ntc-diode-system.yaml`, converted by the ADC |
| 1 kΩ from VDD + diode to VSS | RC2/AN6 | Yes — Shockley model in the same fixture, converted by the ADC |
| Potentiometer POT1 | RC0/AN4 through R32 | No — the fixture drives the same net |
| Push button, mTouch pad, mikroBUS | various | No |
