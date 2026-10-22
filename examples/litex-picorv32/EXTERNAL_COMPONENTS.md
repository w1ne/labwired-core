# External Components — LiteX PicoRV32 SoC

None. The smoke firmware exercises the SoC's own surface only: CPU, SRAM and
the LiteX UART console. The UART is the modelled console and needs no external
device — the engine captures its byte stream directly.

A LiteX board built around this SoC carries more, none of it modelled or
attached in this example:

| Component | Board wiring | State |
| :--- | :--- | :--- |
| Host serial bridge | host serial ↔ the SoC's UART pins | not modelled — the console sink stands in for the bridge |
| SPI configuration flash | LiteX SPI controller ↔ flash chip | controller declared as a stub; the flash window (0x20000000) is mapped memory |
| DDR SDRAM | SoC main RAM (0x40000000) | not mapped |
| Ethernet PHY | LiteEth MAC ↔ PHY | MAC declared as a stub; PHY not modelled |

When any of these becomes a milestone (flash boot, LiteEth console), it goes
here with its fidelity tier and descriptor path, and the system manifest grows
the matching `external_devices` entry.
