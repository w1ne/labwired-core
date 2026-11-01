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

Open item: the A53 cluster's actual counter frequency on silicon is set by
the TI boot flow; 200 MHz is the value every Zephyr AM62x A53 board assumes.
