# Required Documents — FRDM-K64F

Authoritative sources used by this example (retrieved 2026-10-29):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| K64P144M120SF5RM | K64 Sub-Family Reference Manual | SIM (SCGC4/SCGC5), MCG reset clock, UART (BDH/BDL/C1..C5/S1/S2/D), GPIO (PDOR/PSOR/PCOR/PDIR/PDDR), PORT PCR, FTFE memory map |
| K64P144M120SF5 | MK64F12 datasheet | Memory boundaries (1 MiB flash @ 0x0; 64 KiB SRAM_L @ 0x1FFF0000 + 192 KiB SRAM_U @ 0x20000000), 120 MHz max, LQFP100 pinout, I/O DC characteristics |
| FRDM-K64F user manual | NXP Freedom K64 board documentation | Board wiring: RGB LED red PTB22 / green PTE26 / blue PTB21 active low, SW2 PTC6 / SW3 PTA4 active low, OpenSDA VCP on UART0 |
| Zephyr `frdm_k64f` board dts / `nxp/kinetis/k6x/nxp_k6x.dtsi` | Board support documentation | Cross-check of the pin map, UART0 PTB16/PTB17 ALT3, and peripheral bases/IRQs |

The chip descriptor (`configs/chips/mk64f12.yaml`) carries the same values
transcribed with its own citations; this package does not restate them.

Open item: SIM clock gating, PORT pad mux/pulls, MCG PLL/FEE, Ethernet and the
other unmodelled blocks are the explicit L1 boundary, not documentation gaps.
