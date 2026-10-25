# Required Source Documents (NUCLEO-U575ZI-Q)

## MCU (reference manual, datasheet, SVD)

1. STM32U575/585 reference manual **RM0456**:
   https://www.st.com/resource/en/reference_manual/rm0456-stm32u5-series-armbased-32bit-mcus-stmicroelectronics.pdf
2. STM32U575ZI datasheet **DS13736** (2 MiB flash, 768 KiB SRAM + 16 KiB SRAM4,
   160 MHz max):
   https://www.st.com/en/microcontrollers-microprocessors/stm32u575zi.html
3. Vendored CMSIS-SVD register database (bases, IRQs, reset values) —
   `tests/fixtures/real_world/stm32u575.svd`,
   sha256 `2e45c70a3387a46a07092b145301e7bee3e95480377e67e252db6bf4ecfd701a`:
   https://raw.githubusercontent.com/cmsis-svd/cmsis-svd-data/main/data/STMicro/STM32U575.svd
4. STM32U5 CMSIS device header (chip header used by the HAL build):
   https://github.com/STMicroelectronics/cmsis-device-u5/blob/main/Include/stm32u575xx.h

## Board (NUCLEO-U575ZI-Q)

1. Board user manual **UM2861** (MB1549: LEDs, VCP wiring):
   https://www.st.com/resource/en/user_manual/um2861-stm32u575ziq-nucleo-64-board-mb1549-stmicroelectronics.pdf
2. STM32U5xx Nucleo BSP (`COM1` = USART1 PA9/PA10, LD1/LD2/LD3):
   https://github.com/STMicroelectronics/stm32u5xx-nucleo-bsp
3. Zephyr board documentation `nucleo_u575zi_q` (console = USART1,
   `led0` = blue LD2, clock tree):
   https://docs.zephyrproject.org/3.7.0/boards/st/nucleo_u575zi_q/doc/index.html

## Cross-simulator reference (Renode-derived, not silicon)

Upstream Renode master has no STM32U5 platform (checked 2026-09-17,
`platforms/cpus/` listing); closest upstream references are `stm32wba52.repl`
and `stm32l552.repl`. The community **60North `renode-stm32u5`** package fills
that gap and is the behavioural acceptance reference used here:

- 60North `renode-stm32u5` at commit `9cef86b9a` (2026-10-22), Renode 1.17.0
  (reference worktree: `/home/andrii/projects/renode-stm32u5-60north`)
- STM32CubeU5 **v1.9.0**, pinned by 60North at
  `d88042df24f16799957c24f00c2b234c9e306188`:
  https://github.com/STMicroelectronics/STM32CubeU5/tree/v1.9.0
- ST Nucleo BSP `stm32u5xx-nucleo-bsp` v1.2.3 (`ed50f1d2…`) and the
  STM32U575I-EV BSP `stm32u575i-ev-bsp` v1.4.1 (`83d8fb77…`).

**This is a Renode-derived reference, not silicon.** The 60North suite is a
second implementation of the same reference manuals; agreement with it is
evidence that LabWired matches the reference model's documented behaviour, not
evidence against a bench part. Every value in the chip YAML remains
SVD/RM0456/DS13736-derived, and no silicon differential is claimed. See
[`../../validation/stm32u5-60north/README.md`](../../validation/stm32u5-60north/README.md)
for the reproduction commands and expected result symbols.
