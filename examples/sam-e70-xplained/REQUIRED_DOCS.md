# Required Documents — SAM E70 Xplained

Authoritative sources used by this example (retrieved 2026-10-30):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| DS60001527 | SAM E70/S70/V70/V71 Family Data Sheet (Microchip) | PIO, PMC, USART, WDT and bus-matrix register layouts (PIO_ABCDSR1/2 @ +0x70/+0x74, PMC_PCER1 @ +0x100, US_BRGR CD/FP, WDT_MR.WDDIS, CCFG_SYSIO.SYSIO4); the 12 MHz main RC at reset |
| Zephyr `dts/arm/atmel/samx7x.dtsi`, `samx7xx21.dtsi` | SoC devicetree (main) | Peripheral bases and interrupt numbers (PMC 0x400E0600 / 5, UART0-4, EEFC, PIOA-E / 10, 11, 12, 16, 17, RSTC/SUPC/WDT/RTC, USART0-2 0x40024000+ / 13-15); flash 2 MiB at 0x00400000, SRAM 384 KiB at 0x20400000 |
| Zephyr `boards/atmel/sam/sam_e70_xplained/sam_e70_xplained-common.dtsi`, `-pinctrl.dtsi` | Board support (main) | LED0 on PC8 active low; SW0 on PA11 active low with pull-up; console `usart1` at 115200, RXD PA21 (periph A) / TXD PB4 (periph D) |
| Renode v1.17.0 `platforms/cpus/sam_e70.repl` | Renode SAM E70 platform | Cross-check of the USART0-2, GEM, TRNG and PMC_SR addresses. Not a parity claim; its 256 MiB flash/RAM sizes are not used |

The chip descriptor (`configs/chips/same70q21b.yaml`) carries the same values
transcribed with its own citations; this package does not restate them.

Open item: clock generation, the watchdog timeout, the PIO peripheral mux and
the unmapped blocks are the explicit L1 boundary, not documentation gaps.
