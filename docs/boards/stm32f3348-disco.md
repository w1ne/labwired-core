# STM32F3348-DISCO

| Field | Value |
|---|---|
| Board | ST STM32F3348-DISCO (MB1142) |
| MCU | STM32F334C8T6, GPIO smoke slice |
| System profile | [`configs/systems/stm32f3348-disco.yaml`](../../configs/systems/stm32f3348-disco.yaml) |
| Chip profile | [`configs/chips/stm32f334c8-f3348-disco-smoke.yaml`](../../configs/chips/stm32f334c8-f3348-disco-smoke.yaml) |
| Firmware | [`crates/firmware-stm32f3348-disco-smoke/`](../../crates/firmware-stm32f3348-disco-smoke/) |
| Smoke and evidence | [`examples/stm32f3348-disco/`](../../examples/stm32f3348-disco/) |

The smoke boots the modeled Cortex-M4 and toggles red user LED LD3 on PB6.
The focused model includes only RCC and GPIOB for this path. It omits the other
LEDs, user button, ST-LINK/VCP, high-brightness LED driver, and remaining MCU
peripherals. ST's live vendor status and the smoke boundary are recorded in
the example evidence.
