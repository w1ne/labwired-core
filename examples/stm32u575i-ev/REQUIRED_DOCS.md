# Required Documents — STM32U575I-EV

Authoritative sources used by this example:

## Board

1. **UM2854 Rev 2** (May 2024), *Evaluation board with STM32U575AI MCU*:
   https://www.st.com/resource/en/user_manual/dm00779680.pdf
   — MB1550 main board, STM32U575AII6Q (2 MiB flash / 786 KiB SRAM total),
   ordering information.
2. ST **`stm32u575i-ev-bsp` v1.4.1** (`83d8fb77a80a4c6d458f37f6ec10471ba95ddc96`),
   `stm32u575i_eval.h` / `stm32u575i_eval.c`:
   https://github.com/STMicroelectronics/stm32u575i-ev-bsp
   — LED5 = PB7, LED6 = PH7 (active high, `BSP_LED_On` drives high); LED7/LED8
   behind the MFX I/O expander; USER button PC13 (active high, external
   pull-down); COM1 = USART1 PA9/PA10 (AF7).
3. STM32CubeU5 **v1.9.0**, pinned by 60North at
   `d88042df24f16799957c24f00c2b234c9e306188`:
   https://github.com/STMicroelectronics/STM32CubeU5/tree/v1.9.0
   — the BSP revision and the unmodified `FDCAN_Loopback` example used by the
   60North reference.

## MCU (shared with the NUCLEO-U575ZI-Q)

1. **RM0456**, STM32U575/585 reference manual (RCC, PWR, FLASH, GPIO, USART):
   https://www.st.com/resource/en/reference_manual/rm0456-stm32u5-series-armbased-32bit-mcus-stmicroelectronics.pdf
2. **DS13736**, STM32U575ZI datasheet (2 MiB flash, 768 KiB SRAM + 16 KiB
   SRAM4, 160 MHz):
   https://www.st.com/en/microcontrollers-microprocessors/stm32u575zi.html
3. Vendored CMSIS-SVD `tests/fixtures/real_world/stm32u575.svd`, sha256
   `2e45c70a3387a46a07092b145301e7bee3e95480377e67e252db6bf4ecfd701a`.
4. ST CMSIS device header:
   https://github.com/STMicroelectronics/cmsis-device-u5/blob/main/Include/stm32u575xx.h

## Cross-simulator reference (Renode-derived, not silicon)

The 60North **`renode-stm32u5`** package (commit `9cef86b9a`, 2026-10-22,
Renode 1.17.0) provides the STM32U575 platform and the ST example firmware the
`stm32u575_60north` target compares against. It is a Renode-derived reference,
**not silicon**. Its `platforms/boards/stm32u575i_ev.repl` names the same LED5
PB7 / LED6 PH7 wiring, and `tests/boards/stm32u575i-ev-st-examples.robot` is the
FDCAN loopback acceptance case. See
[`../../validation/stm32u5-60north/README.md`](../../validation/stm32u5-60north/README.md).
