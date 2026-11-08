# NUCLEO-H755ZI-Q

| Field | Value |
|---|---|
| Board | ST NUCLEO-H755ZI-Q (MB1363) |
| MCU | STM32H755ZIT6, modeled Cortex-M7 smoke slice |
| System profile | [`configs/systems/nucleo-h755zi-q.yaml`](../../configs/systems/nucleo-h755zi-q.yaml) |
| Chip profile | [`configs/chips/stm32h755zi-nucleo-m7-smoke.yaml`](../../configs/chips/stm32h755zi-nucleo-m7-smoke.yaml) |
| Firmware | [`crates/firmware-nucleo-h755zi-q-smoke/`](../../crates/firmware-nucleo-h755zi-q-smoke/) |
| Smoke and evidence | [`examples/nucleo-h755zi-q/`](../../examples/nucleo-h755zi-q/) |

The local firmware smoke boots on the Cortex-M7 model and toggles green user
LED LD1 on PB0. The narrow profile includes only the memory range and RCC/GPIOB
models used by this firmware. It does not claim the second Cortex-M4 core or
the board's other peripherals. ST's live status and the smoke boundary are
recorded in the example evidence.
