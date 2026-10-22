# Required Documents — LiteX Minerva SoC

Authoritative sources used by this example (retrieved 2026-10-22):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| LiteX documentation | `litex.readthedocs.io` | SoC memory-map conventions (integrated memory, SRAM, main RAM, CSR space), the BIOS + boot-flash load flow this example skips |
| LiteX source | `litex/soc/cores/uart.py` | The UART core's 32-bit CSR layout: `rxtx` @ +0x00, `txfull` @ +0x04, `rxempty` @ +0x08 — the map the engine's Litex UART profile models |
| Pinned LiteX Minerva platform | the upstream SoC definition | Minerva RV32I core, 100 MHz, CSR base 0xE0000000, UART @0xE0001800 (IRQ 0), timer @0xE0002800 (IRQ 1), Ethernet @0xE0003800 (IRQ 2), ROM @0x20020000 (32 KiB), boot flash @0x20228000 (1 MiB), integrated memory @0x0 (256 KiB), SRAM @0x10000000 (256 KiB), DRAM @0x40000000 |
| Minerva (SpinalHDL) | CPU core documentation | RV32I — no multiply/divide, no compressed instructions; the smoke is built for `riscv32i-unknown-none-elf` |
| `LiteX_Timer` (8-bit CSR variant) | the platform's timer model | The register map differs from the 32-bit CSR timer modelled for the VexRiscv SoC (byte-wide LOAD0..3/RELOAD0..3/VALUE0..3 plus the event CSRs), so this block is declared a stub until a variant-specific model lands |

Open items: the timer and Ethernet MAC are declared stubs; the LiteX BIOS and
boot-flash load are out of scope for fast boot; main RAM is not mapped.
