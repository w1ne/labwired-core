# Required Documents — S32K388EVB-Q289

Authoritative sources used by this example (retrieved 2026-10-30):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| NXP RTD `BaseNXP` device headers `S32K388_*.h` | S32K388 device headers (Real-Time Drivers) | Peripheral bases (`IP_SIUL2_BASE`, `IP_LPUART_n_BASE`, `IP_MC_ME_BASE`, `IP_SWT_0_BASE`, …), `IRQn_Type`, and the SIUL2 (`S32K388_SIUL2.h`), LPUART (`S32K388_LPUART.h`), MC_ME and SWT register layouts |
| NXP `S32K388_M7.svd` | S32K388 SVD | MC_CGM `MUX_0` / SWT reset values (FIRC selected, `DC_0` = /1) |
| S32DS RTD template `linker_flash_s32k388.ld` | S32 Design Studio linker file | ITCM/DTCM, 8 MiB program flash at `0x0040_0000` (last 176 KiB HSE), data flash, SRAM_0–2 at `0x2040_0000` |
| S32DS RTD template `ivt_config.c` | Boot header layout | IVT marker `0x5AA55AA5`, `BOOT_CONFIG`, CM7_0 start address at IVT + `0x0C` |
| Zephyr `drivers/gpio/gpio_nxp_siul2.c` | SIUL2 GPIO driver (main) | Bit order of the parallel registers (`PGPDO`/`PGPDI` bit `15 - k` is pin `16·p + k`) |
| NXP "Start Developing With S32K388EVB-Q289" | Board quick start | Green LED on PTF23 |
| Renode v1.17.0 `platforms/cpus/nxp-s32k388.repl`, `platforms/boards/nxp-s32k388evb.repl`, `tests/platforms/S32K388.robot` | Renode S32K388 platform | Cross-check of the SIUL2/LPUART/SWT/MC_ME/MC_CGM bases and LPUART IRQs 141–156; the other RGB LED pads (PTF17–19, PTF22, PTF24), SW10 on PTA1 and LPUART2 as the console. Not a parity claim |

The chip descriptor (`configs/chips/s32k388.yaml`) carries the same values
transcribed with its own citations; this package does not restate them.

Open items:

- The EVB hardware user manual was not available, so the LED polarity, the
  RGB pads other than PTF23 and SW10 on PTA1 come from the Renode board file
  only.
- Which LPUART and pads the EVB's USB-UART bridge uses is not confirmed;
  LPUART2 is the instance the Renode S32K388 test reads.
