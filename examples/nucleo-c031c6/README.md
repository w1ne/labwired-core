# NUCLEO-C031C6

This narrow profile boots firmware for the STM32C031C6 and exercises LD4,
the active-high user LED on PA5. It exposes only the reset-clock boot path,
RCC_IOPENR GPIOA clock gate, and GPIOA. The user button, VCP, USB, and all
other MCU peripherals are omitted.

Reproduce from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-c031c6 \
  --crate firmware-nucleo-c031c6-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-c031c6/onboarding-smoke.yaml \
  --system configs/systems/nucleo-c031c6.yaml \
  --out-dir examples/nucleo-c031c6/evidence
```

The committed build and simulator output is under `evidence/`. ST's live status
and stock check is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Sources and model boundary

- ST [NUCLEO-C031C6 product page](https://www.st.com/en/evaluation-tools/nucleo-c031c6.html): active product status.
- ST [UM2953 Rev 2](https://www.st.com/resource/en/user_manual/um2953-stm32c0-nucleo64-board-mb1717-stmicroelectronics.pdf): the board's LD4 user LED is connected to PA5 and lights when PA5 is high.
- ST [STM32C031C6 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32c031c6.html): Cortex-M0+, 32 KiB Flash, 12 KiB SRAM, up to 48 MHz.
- ST [RM0490 Rev 5](https://www.st.com/resource/en/reference_manual/rm0490-stm32c0-series-advanced-armbased-32bit-mcus-stmicroelectronics.pdf): C0 RCC_IOPENR at offset 0x34 and GPIOAEN at bit 0.

The smoke checks GPIO transitions on PA5. The chip descriptor uses the C0 RCC
and STM32 GPIO models; it declares no zero-answering peripheral stubs. This is
local repository-engine evidence, not a hosted `model_verified` result or a
physical-board capture. Power-on and brown-out reset behavior is outside the
profile.
