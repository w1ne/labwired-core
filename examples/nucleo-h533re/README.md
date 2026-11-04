# NUCLEO-H533RE

This focused profile boots Cortex-M33 firmware on the STM32H533RET6 and
exercises the board's active-high user LED LD2 on PA5. It exposes the H5 RCC,
GPIOA, SysTick, and the 128 KiB SRAM1 bank used by the smoke. The remaining
H533 SRAM banks, TrustZone configuration, VCP, button, USB, and other MCU
peripherals are outside the profile.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-h533re \
  --crate firmware-nucleo-h533re-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-h533re/onboarding-smoke.yaml \
  --system configs/systems/nucleo-h533re.yaml \
  --out-dir examples/nucleo-h533re/evidence
```

The committed build and simulation output is in `evidence/`. ST's live status
and purchase check is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources and model boundary

- ST [UM3121 Rev 6](https://www.st.com/resource/en/user_manual/um3121-stm32h5-nucleo64-board-mb1814-stmicroelectronics.pdf): the NUCLEO-H533RE is an MB1814 H5 Nucleo-64 variant; LD2 is active-high on PA5.
- ST [DS14539](https://www.st.com/resource/en/datasheet/stm32h533he.pdf): STM32H533RE memory and Cortex-M33 profile.
- ST [RM0481 Rev 2](https://www.st.com/resource/en/reference_manual/rm0481-stm32h533-stm32h563-stm32h573-and-stm32h562-armbased-32bit-mcus-stmicroelectronics.pdf): H533 RCC and GPIO address aliases and register definitions.

The smoke checks GPIO transitions through the local engine. It does not verify
TrustZone attribution or boot configuration, cryptography, button input, VCP,
USB, or other H533 peripherals. The run records `unpowered_rail_assumed`, so
physical supply sequencing and brown-out behavior are outside this evidence.
