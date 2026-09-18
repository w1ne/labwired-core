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

## Cross-simulator reference (honest gap)

Renode master has no STM32U5 platform (checked 2026-09-17, `platforms/cpus/`
listing); closest references `stm32wba52.repl`, `stm32l552.repl`. No
Renode-vs-LabWired differential is claimed for this part — every value in the
chip YAML is SVD/RM0456/DS13736-derived.
