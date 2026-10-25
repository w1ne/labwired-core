# Required Documents — Digilent Arty A7-35T (LiteX VexRiscv SoC)

Authoritative sources used by this example (retrieved 2026-10-19 / 2026-10-22):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| Digilent Arty A7 reference manual | Arty A7-35T board reference | Board facts: 100 MHz oscillator, four user LEDs LD4–LD7, four slide switches SW0–SW3, four push-buttons BTN0–BTN3, the FTDI USB-UART bridge, the SPI configuration flash and DDR3 |
| LiteX documentation | `litex.readthedocs.io` | SoC memory-map conventions (integrated ROM 0x0, SRAM 0x10000000, main RAM 0x40000000, CSR space), the BIOS + flash-load boot flow this example skips |
| LiteX source | `litex/soc/cores/uart.py`, `litex/soc/cores/timer.py` | The UART core's CSR layout (`rxtx`/`txfull`/`rxempty`) and the countdown timer's register map + zero-event CSRs |
| LiteX gateware for the Arty | public VexRiscv SoC build for the Arty | The 100 MHz clock, UART IRQ 2, timer IRQ 1, and the CSR page assignment this descriptor's 0x60000000-based map reproduces |
| VexRiscv (SpinalHDL) | CPU core documentation | `rv32im_zicsr_zifencei` — no compressed/atomic extensions, so the smoke is built for `riscv32im-unknown-none-elf` |
| Control-and-Status CSR layout | the gateware's generated `csr.h` | `Leds` @0x00 (R/W), `Switches` @0x04, `ButtonsStatus` @0x08, `ButtonsPending` @0x0C (W1C), `ButtonsEnabled` @0x10; pads: LEDs 0–31, switches 32–63, buttons 64–95 |

The CSR base is a gateware parameter (0x60000000 / 0xE0000000 / 0xF0000000
appear across LiteX builds); this chip pins the 0x60000000-based map and
documents the shadow window rather than modelling two aliases.

Open items: the SPI-flash controller, Ethernet MAC and MMCM are declared
stubs; the LiteX BIOS and flash-load boot flow are out of scope for fast
boot; the board's buttons are active-low and the CAS descriptor carries
`buttons_active_low: true`.
