# Required Documents — LiteX PicoRV32 SoC

Authoritative sources used by this example (retrieved 2026-10-22):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| LiteX documentation | `litex.readthedocs.io` | SoC memory-map conventions (integrated memory, SRAM, main RAM, CSR space, SPI-flash window), the BIOS + flash-load flow this example skips |
| LiteX source | `litex/soc/cores/uart.py` | The UART core's 32-bit CSR layout: `rxtx` @ +0x00, `txfull` @ +0x04, `rxempty` @ +0x08 |
| Pinned LiteX PicoRV32 platform | the upstream SoC definition | PicoRV32 (`rv32imc`), 100 MHz, CSR base 0x60000000 (shadow 0xE0000000), UART @0x60001800 (IRQ 3), timer0 @0xE0002000 (IRQ 4), Ethernet @0x60007800 (IRQ 5), integrated memory 0x0 (256 KiB), SRAM 0x10000000 (256 KiB), SPI-flash window 0x20000000 (32 MiB) |
| PicoRV32 (YosysHQ) | CPU core documentation | RV32IMC — multiply and compressed instructions present; the smoke is built for `riscv32imc-unknown-none-elf` and issues a real multiply |
| `LiteX_Timer` (8-bit CSR variant) | the platform's timer model | Byte-wide LOAD0..3/RELOAD0..3/VALUE0..3 plus the event CSRs, unlike the 32-bit CSR timer modelled for the VexRiscv SoC — so this block is declared a stub |

Open items: the timer, SPI-flash controller and Ethernet MAC are declared
stubs; the LiteX BIOS and flash load are out of scope for fast boot; main RAM
is not mapped.
