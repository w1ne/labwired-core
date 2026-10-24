# External Components — LiteX VexRiscv SoC

None. The smoke firmware exercises the SoC's own surface only: CPU, SRAM,
and the LiteX UART console. The UART is the modelled console and needs no
external device — the engine captures its byte stream directly.

A LiteX board built around this SoC carries more, none of it modelled or
attached in this example:

| Component | Board wiring | State |
| :--- | :--- | :--- |
| USB-UART bridge (FTDI on an Arty A7) | host serial ↔ the SoC's UART pins | not modelled — the console sink stands in for the bridge |
| SPI configuration flash | LiteX SPI controller ↔ flash chip | controller declared as a stub; the flash window (0x20000000) is mapped memory |
| DDR3 SDRAM | SoC main RAM (0x40000000) | not mapped — see `VALIDATION.md` |
| Ethernet PHY | LiteEth MAC ↔ PHY (MII) | MAC declared as a stub; PHY not modelled |
| LEDs / switches / buttons | gateware-specific control/status CSRs | not modelled — no board_io bindings are declared |

When any of these becomes a milestone (SPI-flash boot, LiteEth console,
LED/switch demo), it goes here with its fidelity tier and descriptor path,
and the system manifest grows the matching `external_devices` or `board_io`
entry.
