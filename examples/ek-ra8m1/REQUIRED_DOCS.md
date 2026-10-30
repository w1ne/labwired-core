# Required Documents — EK-RA8M1

Authoritative sources used by this example (retrieved 2026-10-30):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| `R7FA8M1AH.h` | Renesas FSP CMSIS device header (`renesas/fsp`, `ra/fsp/src/bsp/cmsis/Device/RENESAS/Include/`) | Peripheral bases (`R_SYSTEM` 0x4001E000, `R_MSTP` 0x40203000, `R_PORT0` 0x40400000, `R_PFS` 0x40400800, `R_PMISC` 0x40400D00, `R_SCI_B0` 0x40358000, `R_ICU` 0x40006000, stub windows); `R_SCI_B0_Type` offsets and bit fields (RDR, TDR, CCR0-4, CESR, FCR, CSR, CFCLR); SYSTEM offsets (`SCICKDIVCR` +0x54, `SCICKCR` +0x55 with CKSREQ bit 6 / CKSRDY bit 7, `PRCR` +0x3FA); `MSTPCRB` +0x04; `PWPR` PMISC+0x0C, `PWPRS` PMISC+0x14; PmnPFS PSEL/PMR/PCR positions |
| FSP `ra/fsp/src/bsp/mcu/ra8m1/bsp_feature.h` | FSP RA8M1 feature table | Code flash start 0x02000000; reset `SCKDIVCR` = 0 and ICLK /1; SCI has its own clock (`BSP_FEATURE_SCI_HAS_CLOCK`); non-secure offset 0x10000000 |
| FSP `bsp_clocks.c`, `bsp_module_stop.h`, `bsp_register_protection.c` | FSP BSP sources | The `bsp_peripheral_clock_set` CKSREQ/CKSRDY sequence; SCIn stop bit = `MSTPCRB` bit (31 − n); PRCR key 0xA500 / PRC0 |
| FSP `r_sci_b_uart.c`, `r_uart_api.h`, `r_ioport.h` | FSP SCI_B UART driver and enums | `CCR2` reset 0xFF00FF04 and `FCR` reset 0x1F1F0000; the open order (CCR0 off, CFCLR 0x9D070010, CCR1-3, TE/RE, wait CESR.TIST/RIST); `UART_DATA_BITS_8` = 2; PSEL `SCI1_3_5_7_9` = 0x05 |
| Zephyr `boards/renesas/ek_ra8m1/ek_ra8m1.dts`, `ek_ra8m1-pinctrl.dtsi` | Board support (main) | LED1 P600 / LED2 P414 / LED3 P107 active high; S1 P009 / S2 P008 active low with pull-up; console `uart9` on SCI9, TX PA14 / RX PA15 |
| Zephyr `dts/arm/renesas/ra/ra8/ra8x1.dtsi`, `r7fa8m1xh.dtsi` | SoC devicetree (main) | SRAM 896 KiB at 0x22000000, code flash at 0x02000000 (2016 KiB exposed), data flash 12 KiB at 0x27000000, sci0-9 `renesas,ra8-uart-sci-b`, Cortex-M85 |
| Renode v1.17.0 `platforms/cpus/renesas-r7fa8m1a.repl`, `platforms/boards/renesas-ek_ra8m1.repl`; renode-infrastructure `Peripherals/SCI/RenesasRA8M1_SCI.cs` (MIT) | Renode RA8M1 platform and SCI_B model | Cross-check of the memory map (ITCM/DTCM 64 KiB, code flash 2 MiB, SRAM 0xE0000) and of CESR TIST/RIST mirroring CCR0 TE/RE. Not a parity claim. Renode's ICU base (0x4000C000) and red LED (port9 13) disagree with FSP/Zephyr; FSP/Zephyr are used |

The chip descriptor (`configs/chips/r7fa8m1ah.yaml`) carries the same values
transcribed with its own citations; this package does not restate them.

Not consulted for this slice: the RA8M1 Group User's Manual: Hardware
(R01UH0994). The SCI_B async baud formula used in the firmware
(`N = SCICLK / (64 · 2^(2n−1) · B) − 1`) is the RA SCI formula the RA4M1/RA6M5
packages already cite, and the reset values the model uses come from the FSP
driver defaults, not the manual. The MOCO frequency (8 MHz) is from the Zephyr
`r7fa8m1xh.dtsi` `clock-moco` node.

Open item: clock generation, module stop, ICU event linking, TrustZone,
Helium/MVE and the unmodelled blocks are the explicit L1 boundary, not
documentation gaps.
