# NUCLEO-L496ZG-P

| Field | Value |
|---|---|
| Board | ST NUCLEO-L496ZG-P (MB1312) |
| MCU | STM32L496ZGT6PU |
| System profile | [`configs/systems/nucleo-l496zg-p.yaml`](../../configs/systems/nucleo-l496zg-p.yaml) |
| Chip profile | [`configs/chips/stm32l496zg-nucleo-smoke.yaml`](../../configs/chips/stm32l496zg-nucleo-smoke.yaml) |
| Firmware | [`crates/firmware-nucleo-l496zg-p-smoke/`](../../crates/firmware-nucleo-l496zg-p-smoke/) |
| Smoke and evidence | [`examples/nucleo-l496zg-p/`](../../examples/nucleo-l496zg-p/) |

The local firmware smoke boots on the Cortex-M4 model and toggles the red
on-board LD3 through GPIOB pin 14. The profile contains the L4 RCC and GPIOB
models needed for this path. Other board features and MCU peripherals are not
claimed; the exact board variant, vendor status, and omitted capabilities are
documented in the example evidence.
