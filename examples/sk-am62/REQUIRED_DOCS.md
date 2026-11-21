# Required Documents — SK-AM62

Sources used by this example (retrieved 2026-11-01):

| Doc | Used for |
| :--- | :--- |
| Arm Architecture Reference Manual for A-profile (DDI 0487) | A64 encodings and pseudocode (AddWithCarry, DecodeBitMasks, the load/store and exclusive forms), the exception model (ESR classes, vector offsets, routing, ERET), stage-1 translation (4 KB granule, descriptors, AP/PXN/UXN, AT/PAR), system register encodings, the generic timer |
| Arm Cortex-A53 MPCore Processor TRM (DDI 0500) | MIDR, ID registers, cache geometry (CCSIDR/CLIDR/CTR), Armv8.0 feature set (no LSE), CPUACTLR/CPUECTLR/L2CTLR |
| Arm GICv3/v4 Architecture Specification (IHI 0069) | Distributor, redistributor (WAKER, TYPER.Last) and CPU-interface system registers (IAR1/EOIR1/PMR/RPR/SGI1R) |
| TI AM62x Technical Reference Manual (SPRUIV7) | Memory map: GIC-600, MAIN_UART0-6, PADCFG, OC SRAM, DDR |
| TI SK-AM62B-P1 user's guide (SPRUJ83) | Board: AM6234, MAIN_UART0 to the FTDI bridge |
| Zephyr v4.3.0 `dts/arm64/ti/ti_am62x_a53.dtsi`, `dts/vendor/ti/k3-am62-main.dtsi`, `boards/ti/sk_am62`, `arch/arm64`, `drivers/interrupt_controller/intc_gicv3.c` | Peripheral bases and SPIs, the 200 MHz counter, the EL3/EL2/EL1 init path, the GIC driver's register use |
| TI AM62x SK EVM User's Guide SPRUJ40F (rev. 08/2026): Table 2-28, Table 2-16 | 40-pin user expansion connector: pin 15 `EXP_GPIO0_32` = ball L23, pin 13 `EXP_GPIO0_42` = ball L21; VDDSHV0 and VDDSHV3 on SoC_DVDD3V3, 3.3 V (retrieved 2026-11-19) |
| TI AM62x data sheet SPRSP58C (rev. 10/2025): Table 5-1, section 6.8.6 | Balls L23 / L21: PADCONFIG33 / 43, mux mode 7 = GPIO0_32 / GPIO0_42, LVCMOS, VDDSHV3; LVCMOS 3.3 V mode VIL max 0.8 V, VIH min 2.0 V (retrieved 2026-11-19) |
| Linux v6.12 `drivers/gpio/gpio-davinci.c`, `arch/arm64/boot/dts/ti/k3-am62-main.dtsi`; Zephyr v4.3.0 `drivers/gpio/gpio_davinci.c` | GPIO register map (`struct davinci_gpio_regs`, `BINTEN` 0x08, banks at 0x10 + 0x28 n, DIR reset 0xFFFF_FFFF); MAIN_GPIO0 0x0060_0000 `ti,ngpio = <92>`, MAIN_GPIO1 0x0060_1000 `ti,ngpio = <52>` (retrieved 2026-11-19) |

Open item: the GPIO PID and PCR registers (0x00 / 0x04) are not transcribed
and read 0. Open item: the A53 cluster's actual counter frequency on silicon is set by
the TI boot flow; 200 MHz is the value every Zephyr AM62x A53 board assumes.
