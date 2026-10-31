# External Components — PIC16F1829 on Curiosity (DM164137)

| Component | Connection | Modelled |
| :--- | :--- | :--- |
| Internal 16 MHz HFINTOSC | (on-chip, CONFIG1 FOSC = INTOSC) | No — `cpu_hz` comes from the manifest; OSCSTAT reads the oscillators ready |
| USB-serial adapter | RB7/TX, RB5/RX on the expansion header | TX only — console capture |
| LED D4 | RA5 | Latch state only (LATA bit 5) |
| Potentiometer, push button, mTouch pad, mikroBUS | various | No |
