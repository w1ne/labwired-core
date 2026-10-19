# Required Documents — LiteX VexRiscv SoC

Authoritative sources used by this example (retrieved 2026-10-19):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| LiteX documentation | `litex.readthedocs.io` | SoC memory-map conventions (integrated ROM at 0x0, SRAM at 0x10000000, main RAM at 0x40000000, memory-mapped SPI-flash window, CSR space), the BIOS + flash-load boot flow this example skips, and the CSR-region model |
| LiteX source | `litex/soc/cores/cpu/vexriscv/core.py` | The VexRiscv CPU's mem_map (`rom` / `sram` / `main_ram` / `csr`) and the CPU variant this descriptor pins (`rv32im_zicsr_zifencei`) |
| LiteX source | `litex/soc/cores/uart.py` | The LiteX UART core's 32-bit CSR layout: `rxtx` @ +0x00 (shared TX/RX), `txfull` @ +0x04, `rxempty` @ +0x08 — the register map the engine's LiteX UART profile models |
| `litex-hub/zephyr-on-litex-vexriscv` | Public LiteX gateware for the Digilent Arty (VexRiscv variant) | The 100 MHz system clock, the external-interrupt assignment (timer0 = 1, UART = 2, Ethernet = 3), and the CSR page assignment (`uart` @ +0x1800, `timer0` @ +0x2800, MMCM @ +0x4800) that this descriptor's CSR base reproduces |
| VexRiscv (SpinalHDL) | CPU core documentation | The `rv32im` ISA without compressed/atomic extensions — the reason the smoke is built for `riscv32im-unknown-none-elf` |
| Digilent Arty A7 reference manual | Board reference (the common carrier for this SoC) | Board-level facts this example does *not* model: the FTDI USB-UART bridge, the SPI configuration flash, DDR3 main RAM, Ethernet PHY |

The CSR base is a gateware parameter, not a chip constant: LiteX builds in
the wild place the CSR region at 0x60000000, 0xE0000000 or 0xF0000000. This
descriptor pins the 0x60000000-based map its CSR addresses are written
against and documents the 0xE0000000 shadow window in the chip comments
rather than modelling two aliases of the same block.

Open items: the timer, SPI-flash controller, Ethernet MAC and MMCM are
declared stubs (see `EXTERNAL_COMPONENTS.md` and `VALIDATION.md`); the
LiteX BIOS and flash-load boot flow are out of scope for fast boot.
