# Required Documents — EK-RA6M5

Authoritative sources used by this example (retrieved 2026-10-30):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| `R7FA6M5BH.h` | Renesas FSP CMSIS device header (`renesas/fsp`, `ra/fsp/src/bsp/cmsis/Device/RENESAS/Include/`) | Peripheral bases (`R_SYSTEM` 0x4001E000, `R_MSTP` 0x40084000, `R_PORT0` 0x40080000, `R_PFS` 0x40080800, `R_PMISC` 0x40080D00, `R_SCI0` 0x40118000); register offsets (`SCKDIVCR` +0x20, `SCKSCR` +0x26, `HOCOCR` +0x36, `OSCSF` +0x3C, `PRCR` +0x3FE, `MSTPCRB` +0x04, `PWPR` +0x03) |
| Zephyr `boards/renesas/ek_ra6m5/ek_ra6m5.dts`, `ek_ra6m5-pinctrl.dtsi` | Board support (main) | LED1-3 on P006-P008 active high; S1 P005 / S2 P004 active low with pull-up; console `uart0` on SCI0, TX P411 / RX P410 |
| Zephyr `dts/arm/renesas/ra/ra6/ra6-cm33-common.dtsi`, `r7fa6m5xh.dtsi`, `r7fa6m5bh3cfc.dtsi` | SoC devicetree (main) | Code flash 2 MiB at 0x0, data flash 8 KiB at 0x08000000, SRAM 512 KiB at 0x20000000, option-setting memory at 0x0100A100+, ioport0-b bases, SCI0 on PCLKA with `MSTPB 31`, IIC/USB/ETHER/GPT bases |
| FSP `ra/fsp/src/bsp/mcu/ra6m5/bsp_feature.h` | FSP RA6M5 feature table | Reset `SCKSCR` = 0x01 (MOCO) and `SCKDIVCR` = 0x22022222 (all /4), hence the smoke's explicit /1 write; SCI clock = PCLKA (`BSP_FEATURE_SCI_CLOCK`) |
| Renode v1.17.0 `platforms/cpus/renesas-r7fa6m5b.repl` | Renode RA6M5 platform | Cross-check of the bus map and the tag ranges used to size the stub windows. Not a parity claim |

The chip descriptor (`configs/chips/r7fa6m5bh.yaml`) carries the same values
transcribed with its own citations; this package does not restate them.

Not consulted for this slice: the RA6M5 Group User's Manual: Hardware
(R01UH0891). The SCI async baud formula (`N = PCLK / (64 · 2^(2n−1) · B) − 1`),
the SCI initialization order (TE/RE off, SMR/SEMR, BRR, one bit time, then
TE/RE) and the PWPR unlock order (B0WI = 0, then PFSWE = 1) are the same as
in the RA4M1 manual R01UH0887 that the RA4M1 package already cites. The MOCO
frequency (8 MHz) is from the Zephyr `r7fa6m5xh.dtsi` `clock-moco` node.

Open item: clock generation, module stop, ICU event linking and the
unmodelled blocks are the explicit L1 boundary, not documentation gaps.
