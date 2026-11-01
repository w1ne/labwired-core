# Required Documents — PolarFire SoC Discovery Kit

Sources used by this example (retrieved 2026-11-01):

| Doc | Used for |
| :--- | :--- |
| RISC-V Unprivileged ISA (20191213) | RV64I `*W` ops and shift widths, M division corner cases (Table 7.1), A LR/SC and AMO semantics, RV64C encodings (Tables 16.5-16.7) |
| RISC-V Privileged Architecture (20211203) | `misa`/`mstatus` WARL fields with M-mode only, trap entry and MRET, `mtvec` vectored mode, interrupt priority MEI > MSI > MTI, `mcycle`/`minstret`, PMP CSR layout on RV64 |
| Microchip PolarFire SoC MSS Technical Reference Manual | MSS memory map (CLINT, PLIC, L2-LIM, MMUART0-4, MSS GPIO/I2C/SPI/QSPI, eNVM, DDR window), E51 + 4×U54 hart numbering |
| Microchip PolarFire SoC FPGA Discovery Kit User Guide | Board: MPFS095T on the kit |
| Zephyr v4.1.0 `dts/riscv/microchip/mpfs.dtsi`, `boards/microchip/mpfs_icicle` | Same MSS as devicetree: ISA strings per hart, CLINT/PLIC/MMUART addresses and IRQs, `reg-shift = <2>`, 150 MHz APB clock, console on MMUART1, `CONFIG_RV_BOOT_HART=1`, 1 MHz `mtime` |
| PC16550D datasheet (TI/National) | LCR.DLAB selecting DLL/DLM at offsets 0/1 |

Open items: SiFive U54 `mvendorid`/`marchid`/`mimpid` values are not
transcribed (they read 0, "not implemented"); the PLIC register map is not
modelled yet.
