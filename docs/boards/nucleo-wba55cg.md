# NUCLEO-WBA55CG

ST's NUCLEO-WBA55CG carries the STM32WBA55CG Cortex-M33 MCU with 1 MiB flash
and 128 KiB SRAM. Its default ST-LINK VCP1 is USART1 on PA8/PB12 (AF7). The
board's user LEDs are LD1 PB4, LD2 PA9, and LD3 PB8; its user buttons are B1
PC13, B2 PB6, and B3 PB7.

The onboarding profile exposes RCC, GPIOA/B/C, SysTick, and USART1. Firmware
prints `NUCLEO_WBA55CG_OK` over USART1 and toggles LD1. The 2.4 GHz radio and
Bluetooth LE/802.15.4 operation are not modeled; neither are USB enumeration,
the ST-LINK bridge, VCP2 on LPUART1, or other unlisted MCU peripherals.

The exact simulator smoke and evidence are in
[`examples/nucleo-wba55cg/`](../../examples/nucleo-wba55cg/). ST's live status
and buyability receipt is
[`vendor-status.md`](../../examples/nucleo-wba55cg/evidence/vendor-status.md).

## Sources

- ST [UM3301 Rev 2](https://www.st.com/resource/en/user_manual/um3301-stm32wba-nucleo64-board-mb1801-and-mb1803-stmicroelectronics.pdf), §§7.4.4, 7.5, 7.6.
- ST [STM32WBA5xxx datasheet DS14127](https://www.st.com/resource/en/datasheet/stm32wba55cg.pdf).
- ST [RM0493](https://www.st.com/resource/en/reference_manual/rm0493-stm32wba52xx-and-stm32wba54xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf).
- ST [NUCLEO-WBA55CG live product status](https://www.st.com/en/evaluation-tools/nucleo-wba55cg.html).
