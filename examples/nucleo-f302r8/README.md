# NUCLEO-F302R8

This focused board profile boots bare-metal firmware on the STM32F302R8T6 and
exercises two modeled board paths: USART2 output through the default ST-LINK
Virtual COM Port and the active-high LD2 user LED on PA5. The system descriptor
declares only these tested board paths.

## Run

```bash
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-f302r8 \
  --crate firmware-nucleo-f302r8-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f302r8/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f302r8.yaml \
  --out-dir examples/nucleo-f302r8/evidence
```

The committed simulator output and metrics are under [`evidence/`](evidence/).
This is a local deterministic simulator smoke, not a physical-board capture.

## Pin map and model scope

| Board connection | MCU pin | Model path |
|---|---|---|
| USART2 TX/RX to ST-LINK VCP | PA2/PA3 | RCC + GPIOA + USART2; smoke checks TX output |
| Green user LED LD2 | PA5 | RCC + GPIOA; smoke checks at least eight output edges |

The chip profile exposes the RCC gate bits used for GPIO/USART2, GPIOA,
SysTick, and USART2. It omits B1, the F302 clock tree, USB, ADC, timers, DMA,
and other untested blocks; it declares no zero-answering stubs.

## Sources

- ST [NUCLEO-F302R8 product page](https://www.st.com/en/evaluation-tools/nucleo-f302r8.html)
  and [live vendor status receipt](evidence/vendor-status.md).
- ST [STM32F302R8 datasheet](https://www.st.com/resource/en/datasheet/stm32f302r8.pdf):
  memory geometry and MCU features.
- ST [UM1724 Rev 17, STM32 Nucleo-64 boards (MB1136)](https://www.st.com/resource/en/user_manual/dm00105823-stm32-nucleo-64-boards-mb1136-stmicroelectronics.pdf):
  NUCLEO-F302R8 board identity, LD2/PA5, and default VCP routing.
- ST [RM0365, STM32F302 reference manual](https://www.st.com/resource/en/reference_manual/dm00094349-stm32f302xb-c-d-e-and-stm32f302x6-8-advanced-arm-based-32-bit-mcus-stmicroelectronics.pdf):
  F302 RCC, GPIO, and USART register layout used by the smoke firmware.
