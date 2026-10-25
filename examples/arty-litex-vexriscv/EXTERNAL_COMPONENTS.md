# External Components — Digilent Arty A7-35T

No external devices are attached in this example. The board's user I/O — four
LEDs, four switches and four push-buttons — is part of the SoC's
Control-and-Status CSR and **is modelled** through `board_io`:

| Board control | CAS register | Binding |
| :--- | :--- | :--- |
| LD4–LD7 (user LEDs) | `Leds` bits 0–3 | `led_ld4` … `led_ld7`, output |
| SW0–SW3 (slide switches) | `Switches` bits 0–3 | `switch_sw0` … `switch_sw3`, input, active low |
| BTN0–BTN3 (push-buttons) | `ButtonsStatus` bits 0–3, latched in `ButtonsPending` | `button_btn0` … `button_btn3`, input, active low |

A LiteX board built around this SoC carries more, none of it modelled here:

| Component | Board wiring | State |
| :--- | :--- | :--- |
| FTDI FT2232HQ USB-UART bridge | host serial ↔ the SoC's UART pins | not modelled — the console sink stands in for the bridge |
| SPI configuration flash | LiteX SPI controller ↔ flash chip | controller declared as a stub; the flash window (0x20000000) is mapped memory |
| DDR3 SDRAM | SoC main RAM (0x40000000) | not mapped |
| Ethernet PHY | LiteEth MAC ↔ PHY (MII) | MAC declared as a stub; PHY not modelled |

When any of these becomes a milestone (SPI-flash boot, LiteEth console), it
goes here with its fidelity tier and descriptor path, and the system manifest
grows the matching `external_devices` entry.
