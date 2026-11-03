# NUCLEO-L452RE-P

ST's NUCLEO-L452RE-P carries an STM32L452RE Cortex-M4 MCU with 512 KiB flash
and 160 KiB SRAM. The board's default ST-LINK VCP uses LPUART1 on PA2/PA3
(AF8), and its LD4 green user LED is active high on PB13.

The LabWired smoke profile exposes only RCC, GPIOA, GPIOB, SysTick and LPUART1.
It does not model the USB/ST-LINK bridge, user button, or other MCU peripherals.
The VCP output is captured from the modeled LPUART1 instance; it does not
simulate USB enumeration or the ST-LINK firmware.

The onboarding firmware prints `NUCLEO_L452RE_P_OK` and toggles LD4. Its
build, smoke output, and onboarding metrics are committed in
[`examples/nucleo-l452re-p/evidence/`](../../examples/nucleo-l452re-p/evidence/).
ST's live product and availability check is in
[`vendor-status.md`](../../examples/nucleo-l452re-p/evidence/vendor-status.md).

## Sources

- ST [UM2206](https://www.st.com/resource/en/user_manual/dm00387966.pdf), §§6.9-6.10: VCP route and LD4 mapping.
- ST [STM32L452xx datasheet DS11912](https://www.st.com/resource/en/datasheet/stm32l452re.pdf): memory size, core, and pin alternate functions.
- ST [RM0394](https://www.st.com/resource/en/reference_manual/rm0394-stm32l41xxx42xxx43xxx44xxx45xxx46xxx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): L4 RCC and peripheral register layout.
