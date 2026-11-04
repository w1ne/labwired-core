# STM32H573I-DK

This focused profile boots Cortex-M33 firmware on the STM32H573IIK3Q and
exercises the board's green user LED LD1 on PI9. The LED is active-low. The
profile exposes the main flash and SRAM ranges, H5 RCC, GPIOI, and SysTick.
The board's other peripherals are outside this smoke profile.

## Reproduce

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32h573i-dk \
  --crate firmware-stm32h573i-dk-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/stm32h573i-dk/onboarding-smoke.yaml \
  --system configs/systems/stm32h573i-dk-smoke.yaml \
  --out-dir examples/stm32h573i-dk/evidence
```

The build and simulator output is committed in [`evidence/`](evidence/).
ST's live product and purchase status is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources and model boundary

- ST [UM3143 Rev 2](https://www.st.com/resource/en/user_manual/dm00953545-.pdf): STM32H573I-DK board wiring; LD1 is active-low on PI9.
- ST [DS14251](https://www.st.com/resource/en/datasheet/stm32h573vi.pdf): H573 core, flash, and SRAM capacity.
- ST [RM0481 Rev 2](https://www.st.com/resource/en/reference_manual/rm0481-stm32h533-stm32h563-stm32h573-and-stm32h562-armbased-32bit-mcus-stmicroelectronics.pdf): H5 RCC and GPIO register map.

The local smoke checks GPIOI transitions. It does not claim TrustZone boot or
model the LCD, Wi-Fi module, USB, Ethernet, audio codec, microSD, Octo-SPI
flash, buttons, or VCP.
