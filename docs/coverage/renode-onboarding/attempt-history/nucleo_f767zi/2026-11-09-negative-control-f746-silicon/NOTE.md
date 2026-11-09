Negative control (2026-11-09): the NUCLEO-F767ZI row with its chip
including stm32f746.yaml directly (F746 geometry 1 MiB / 320 KiB, F746 RCC
masks, SYSCFG profile stm32f7, 24 EXTI lines, no SPI4..6). The board
firmware is linked for 512 KiB, so its initial stack (0x20080000) is
outside the F746 SRAM: HardFault on the first push, BusFault BFAR
0x2007FFFC, FAIL 0/8. The 512 KiB geometry is load-bearing.
