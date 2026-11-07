# NUCLEO-H503RB

This profile boots Cortex-M33 firmware on the STM32H503RBT6 and exercises the
board's active-high user LED LD2 on PA5. The descriptor exposes only RCC,
GPIOA, and SysTick; the VCP, button, USB, and other MCU peripherals are outside
the smoke profile.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-h503rb \
  --crate firmware-nucleo-h503rb-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-h503rb/onboarding-smoke.yaml \
  --system configs/systems/nucleo-h503rb.yaml \
  --out-dir examples/nucleo-h503rb/evidence
```

The committed build and simulation output is in `evidence/`. ST's live status
and purchase check is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources and model boundary

- ST [UM3121 Rev 6](https://www.st.com/resource/en/user_manual/dm00941891.pdf): NUCLEO-H503RB board wiring; LD2 is active-high on PA5.
- ST [DS14053](https://www.st.com/resource/en/datasheet/stm32h503rb.pdf): H503 memory and Cortex-M33 profile.
- ST [RM0492](https://www.st.com/resource/en/reference_manual/rm0492-stm32h503-line-armbased-32bit-mcus-stmicroelectronics.pdf) and [CMSIS device header](https://github.com/STMicroelectronics/cmsis-device-h5/blob/main/Include/stm32h503xx.h): H503 register and memory map.

The smoke checks GPIO output transitions through the local engine. It does not
model or claim button input, VCP output, USB, or the remaining H503 peripherals.
The smoke run records `unpowered_rail_assumed`: supply rails are idealized, so power-on and brown-out reset behavior is not simulated.
