# NUCLEO-H723ZG

This smoke profile boots firmware for the STM32H723ZGT6 and exercises two
board paths: the active-high LD1 on PB0 and the default ST-LINK virtual COM port
through USART3 TX on PD8. The receive pin PD9 is pin-mapped but not exercised.

The descriptor is intentionally narrow. It models the reset-clock RCC, GPIOB,
GPIOD, USART3, and the DTCM region used by the smoke image. Other SRAM banks,
the other user LEDs and button, USB, Ethernet, and the rest of the H723
peripheral estate are not claimed by this profile.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-h723zg \
  --crate firmware-nucleo-h723zg-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-h723zg/onboarding-smoke.yaml \
  --system configs/systems/nucleo-h723zg.yaml \
  --out-dir examples/nucleo-h723zg/evidence
```

The committed build and simulation output is in `evidence/`. ST's live product
and purchase status is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources and model boundary

- ST [UM2407 Rev 6](https://www.st.com/resource/en/user_manual/um2407-stm32h7-nucleo144-board-stmicroelectronics.pdf): NUCLEO-H723ZG is the MB1364 board; LD1 is active-high on PB0 by default, and the default ST-LINK VCP uses USART3 PD8/PD9.
- ST [DS13313](https://www.st.com/resource/en/datasheet/stm32h723zg.pdf): STM32H723ZGT6 memory and Cortex-M7 device facts.
- ST [RM0468](https://www.st.com/resource/en/reference_manual/rm0468-stm32h723-733-725-735-730-and-stm32h730-value-line-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): H7 RCC and peripheral register map.
- ST [STM32H723 CMSIS header](https://github.com/STMicroelectronics/cmsis-device-h7/blob/master/Include/stm32h723xx.h): peripheral base addresses and IRQ numbers.

The smoke run is model evidence, not a physical-board capture. The H7 register
profiles are shared with the H735 model and have not been independently checked
against H723 silicon.
