# NUCLEO-F446ZE onboarding record

## Board and live status

ST listed NUCLEO-F446ZE as **Active / in volume production** on 2026-11-03;
its eStore listing showed **In Stock**, **Add to Cart**, and $25.09 for the
displayed two-unit quantity. The live status and exact URLs are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Modeled board scope

The focused system uses the STM32F446ZET6 profile in
[`configs/systems/nucleo-f446ze.yaml`](../../configs/systems/nucleo-f446ze.yaml).
It exposes and smokes:

- USART3 on PD8/PD9 through the default ST-LINK VCP route.
- LD1 green on PB0, LD2 blue on PB7, and LD3 red on PB14; all active-high.
- MCU blocks used by the firmware: RCC, GPIOB, GPIOD, and USART3.

The firmware prints `NUCLEO_F446ZE_OK` and toggles all three user LEDs. Local
simulator logs and assertion artifacts are under [`evidence/`](evidence/).
The user button, USB, Ethernet, and all other MCU peripherals are outside this
focused profile and remain unsupported here. No physical-board test is claimed.

## Sources

- ST [UM1974](https://www.st.com/resource/en/user_manual/dm00244518-stm32-nucleo144-boardsstmicroelectronics.pdf), §§6.6, 6.9, and Table 12: user-button/LED wiring, default USART3 VCP on PD8/PD9, and LED solder-bridge assignments.
- ST [STM32F446ZE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f446ze.html) and [DS10693](https://www.st.com/resource/en/datasheet/stm32f446zc.pdf): exact STM32F446ZET6 lifecycle, core, and memory.
- ST [RM0390](https://www.st.com/resource/en/reference_manual/rm0390-stm32f446xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): F446 RCC/GPIO/USART3 register map. The focused profile contains only blocks exercised by this smoke.
- Live purchasing status: [`evidence/vendor-status.md`](evidence/vendor-status.md).
