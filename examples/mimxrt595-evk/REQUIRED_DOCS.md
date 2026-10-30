# Required Documents — MIMXRT595-EVK

Authoritative sources used by this example (retrieved 2026-10-30):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| MCUXpresso SDK `devices/MIMXRT595S/MIMXRT595S_cm33.h` | MIMXRT595S device header | Peripheral bases (`FLEXCOMM0_BASE`, `GPIO_BASE`, `IOPCTL_BASE`, `CLKCTL0/1_BASE`, `RSTCTL0/1_BASE`, `SYSCTL0/1_BASE`, …), `IRQn_Type` (`FLEXCOMM0_IRQn` = 14), and the `USART_Type`/`FLEXCOMM_Type`, `GPIO_Type`, `IOPCTL_Type`, `CLKCTL0/1_Type`, `RSTCTL0/1_Type` layouts and bit fields |
| SDK `boards/evkmimxrt595` flash linker file `MIMXRT595Sxxxx_cm33_flash.ld` | Linker file | FCB at `0x0800_0400`, vector table at `0x0800_1000`, data at `0x2028_0000` |
| SDK `boards/evkmimxrt595/board.h` | Board definitions | Debug console FLEXCOMM0 at 115200, RGB LED on P0_14/P1_0/P3_17 (`LOGIC_LED_ON` = 1), SW1 on P0_25, SW2 on P0_10, 64 MiB board flash |
| SDK `devices/MIMXRT595S/system_MIMXRT595S_cm33.c` / `.h` | System init | Main-clock selection (`MAINCLKSELA/B`, `FRODIVSEL`), the 192 MHz FRO and its /2–/16 taps, the 1 MHz LPOSC reset default |
| SDK `devices/MIMXRT595S/drivers/fsl_clock.h` | Clock driver | `kFRO_DIV4_to_FLEXCOMM0` = FCFCLKSEL 0 |
| SDK `devices/MIMXRT595S/drivers/fsl_clock.c` | Clock driver source | The registers `rt5xx_clock_init` polls (`*DIV.REQFLAG`, `FROCLKSTATUS.CLK_OK`, `LPOSCCTL0.CLKRDY`, `SYSPLL0PFD`/`AUDIOPLL0PFD` `PFDn_CLKRDY`, `FRO_CONTROL`/`FRO_CAPVAL`), and the boot-ROM OTP API entry addresses `0x1300_7FFF`/`0x1300_804D`/`0x1300_805D` with their prototypes |
| SDK `drivers/ostimer/fsl_ostimer.c` / `.h` | OS timer driver | `OSTIMER_Type` use: Gray-coded `EVTIMER`/`MATCH`, the L-then-H read, `MATCH_WR_RDY`, `OSEVENT_CTRL` flag (W1C) and enable |
| Zephyr v3.7.0 `dts/arm/nxp/nxp_rt5xx_common.dtsi`, `boards/nxp/mimxrt595_evk`, `soc/nxp/imxrt/imxrt5xx/cm33` | Zephyr board/SoC | The secure address map the stock board uses (flash `0x1800_0000`, SRAM `0x3018_0000`, peripherals `0x5000_0000`), FLEXCOMM12 (IRQ 64), the OS timer (IRQ 41, 1 MHz), and the FlexSPI re-clock in `flash_clock_setup.c` |
| Renode v1.17.0 `platforms/cpus/imxrt500.repl` | Renode RT500 platform | Cross-check of the FLEXCOMM0/12, CLKCTL0/1, OS timer, CTIMER and FlexSPI bases, the 5 MiB SRAM and the 8 MiB FlexSPI window. Not a parity claim |

The chip descriptor (`configs/chips/mimxrt595s.yaml`) carries the same
values transcribed with its own citations; this package does not restate
them.

Open items:

- UM11147 (the i.MX RT500 reference manual) is behind an NXP login and was
  not available; register semantics follow the SDK header and drivers.
- The EVK schematic was not checked, so the SW1/SW2 polarity (taken as
  active-low) is an assumption.
