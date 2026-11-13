# Required Documents — Apollo4 Plus EVB

Authoritative sources used by this example (retrieved 2026-10-30):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| `apollo4p.h` | Ambiq CMSIS device header for the Apollo4 Plus (AmbiqSuite; mirrored in `zephyrproject-rtos/hal_ambiq` `CMSIS/AmbiqMicro/Include/apollo4p.h`) | every peripheral `*_BASE`; `IRQn_Type` (UART0-3 = 15-18); `GPIO_Type` (PINCFGn, PADKEY, RD/WT/WTS/WTC/EN/ENS/ENC, IEOBS/OEOBS, interrupt banks) and its PINCFG bit fields and enums (FNCSEL per pin, INPEN, RDZERO, OUTCFG, PULLCFG, `GPIO_PADKEY_PADKEY_Key` = 0x73); `PWRCTRL_Type` (MCUPERFREQ, DEVPWREN/DEVPWRSTATUS bit positions, AUDSSPWREN); `UART0_Type` (PL011 register layout, CR.CLKEN/CLKSEL, FR bits) |
| `am_reg_base_addresses.h` | hal_ambiq `mcu/apollo4p/regs` | MRAM 2048 KiB at 0x0; TCM 384 KiB at 0x1000_0000; SSRAM0/EXTRAM/SSRAM1 sizes (RAM_TOTAL_SIZE 2816 KiB) |
| Zephyr `dts/arm/ambiq/ambiq_apollo4p.dtsi` | Apollo4 Plus SoC devicetree (main) | application flash at 0x0001_8000 (`flash0`), UART0 IRQ 15, GPIO bank IRQs 56-59, GPIO block size |
| Zephyr `boards/ambiq/apollo4p_evb` (`apollo4p_evb.dts`, `apollo4p_evb-pinctrl.dtsi`, `doc/index.rst`) | Apollo4 Plus EVB board support (main) | LED0-2 on GPIO30/90/97 active low, BTN0/1 on GPIO18/19 active low with pull-up, UART0 TX GPIO60 / RX GPIO47, console 115200 8N1 |
| Renode `AmbiqApollo4_GPIO.cs` | renode-infrastructure (v1.17.0 era) | cross-check only: PINCFG reset value 0x3 (FNCSEL GPIO), PADKEY gating of PINCFG writes, RD gated by INPEN/RDZERO, pins 105-127 virtual |
| Ambiq AMAP4PEVB / AMAP42KP-KBR product listings (Ambiq Apollo4 hardware guide; DigiKey/Mouser) | | board ordering code AMAP4PEVB carries the AMAP42KP-KBR (146-ball BGA) |

The chip descriptor (`configs/chips/amap42kp.yaml`) carries the same values
transcribed with its own citations; this package does not restate them.

Not consulted: the Apollo4 Plus SoC datasheet register chapters directly (the
CMSIS header is generated from the same register database) and the EVB
schematic (the board wiring is taken from the Zephyr board port, which Ambiq
maintains).

Open items: the boot ROM / SBL, CLKGEN, the PL011 `CR` storage gap and the
unmodelled blocks are the explicit L1 boundary, not documentation gaps.
