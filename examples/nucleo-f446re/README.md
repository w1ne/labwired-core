# ST NUCLEO-F446RE

The NUCLEO-F446RE is an active STM32 Nucleo-64 board built around the
STM32F446RET6. The demand brief places this under **Other STM32** (#4, 13% of
MCU use); STM32F4 is #7 (8%), and Nucleo/Discovery boards appear in 23% of
respondents' main project kits.

This focused system profile models the F446RE Cortex-M4, the F4 RCC block,
GPIOA, and USART2. It maps LD2 to PA5 and the default ST-LINK VCP to USART2 on
PA2/PA3. ST-LINK USB/debug, external clock sources, the user button, and the
other F446 peripherals are outside this profile.

## Copy-paste STM32Cube HAL blink

Select `NUCLEO-F446RE` in STM32CubeIDE or CubeMX, then use this `main.c` body
after the generated clock setup:

```c
#include "main.h"

int main(void) {
  HAL_Init();
  __HAL_RCC_GPIOA_CLK_ENABLE();

  GPIO_InitTypeDef led = {0};
  led.Pin = GPIO_PIN_5;
  led.Mode = GPIO_MODE_OUTPUT_PP;
  led.Pull = GPIO_NOPULL;
  led.Speed = GPIO_SPEED_FREQ_LOW;
  HAL_GPIO_Init(GPIOA, &led);

  while (1) {
    HAL_GPIO_TogglePin(GPIOA, GPIO_PIN_5);
    HAL_Delay(500);
  }
}
```

## Run the simulator smoke

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-f446re-usart2-gpio \
  --crate firmware-nucleo-f446re-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f446re/io-smoke.yaml \
  --system configs/systems/nucleo-f446re.yaml \
  --out-dir examples/nucleo-f446re/evidence
```

The smoke requires the USART2 marker and at least eight PA5 edges. It leaves
the MCU on reset HSI at 16 MHz. Results and dated vendor status are recorded
under [`evidence/`](evidence/) and in [vendor-status.md](vendor-status.md).
