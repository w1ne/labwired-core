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
| Microchip PolarFire SoC Data Sheet DS00004248E (03/2026), Table 4-15 | LVCMOS33 DC input levels: VIL max 0.8 V, VIH min 2.0 V (retrieved 2026-11-19) |
| `polarfire-soc/polarfire-soc-discovery-kit-reference-design` @a84023d1: `MPFS_DISCOVERY_KIT_MSS.cfg`, `components/MPFS_DISCOVERY_KIT.tcl`, `constraints/MPFS_DISCOVERY_RPi.pdc`, `constraints/MPFS_DISCOVERY_KIT_BANK_SETTINGS.pdc` | MSS GPIO_2 on the fabric; GPIO_2_0_IO = RPI_GPIO4 (E12), GPIO_2_1_IO = RPI_GPIO17 (G18), LVCMOS33; Bank 1 at 3.3 V with J45 on 1-2 (retrieved 2026-11-19) |
| Microchip `mss_gpio.h` / `mss_gpio.c` (polarfire-soc/platform) | `GPIO_TypeDef` offsets (GPIO_CFG[n] 4n, GPIO_IN 0x84), `MSS_GPIO_INPUT_MODE` = 0x02, bank widths 14/24/32 |

Open items: SiFive U54 `mvendorid`/`marchid`/`mimpid` values are not
transcribed (they read 0, "not implemented"); the PLIC register map is not
modelled yet.
