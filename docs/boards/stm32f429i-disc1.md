# STM32F429I-DISC1

| Field | Value |
|---|---|
| Board | ST STM32F429I-DISC1 (MB1075) |
| MCU | STM32F429ZIT6, GPIO smoke slice |
| System profile | [`configs/systems/stm32f429i-disc1.yaml`](../../configs/systems/stm32f429i-disc1.yaml) |
| Chip profile | [`configs/chips/stm32f429zi-f429i-disc1-smoke.yaml`](../../configs/chips/stm32f429zi-f429i-disc1-smoke.yaml) |
| Firmware | [`crates/firmware-stm32f429i-disc1-smoke/`](../../crates/firmware-stm32f429i-disc1-smoke/) |
| Smoke and evidence | [`examples/stm32f429i-disc1/`](../../examples/stm32f429i-disc1/) |

The smoke boots the modeled Cortex-M4 and toggles green user LED LD3 on PG13.
The focused profile includes reset-clock RCC and GPIOG only. It omits the LCD,
SDRAM, gyroscope, USB OTG, buttons and ST-LINK/VCP. ST listed the board and
STM32F429ZI as Active; its eStore showed the board In Stock on 2026-11-04. The
local simulator evidence does not include a physical board capture or a hosted
`labwired_verify` verdict.
