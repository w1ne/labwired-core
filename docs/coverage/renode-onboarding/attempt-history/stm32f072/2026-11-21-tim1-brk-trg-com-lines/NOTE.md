# TIM1/TIM8 break and trigger/commutation lines (2026-11-21)

## Result
TIF, COMIF and BIF now interrupt. On split-vector chips they pend
TIMx_TRG_COM (UP + 1) and TIMx_BRK (UP - 1). On F0/G0/C0 they pend the
single TIMx_BRK_UP_TRG_COM line.

## Defects
1. Timer model: the IRQ mask was `0x1F` (UIF, CC1..4IF). TIE, COMIE and
   BIE never pended a line, on any timer.
2. Descriptors: no `trigger_com_irq` / `break_irq` (new keys). 30 timers
   on 27 descriptors now name them. Vector numbers are from the vendor
   CMSIS headers (stm32f103x6.h, stm32f401xc.h, stm32f411xe.h,
   stm32l432xx.h, stm32l476xx.h, stm32l552xx.h, stm32g431xx.h,
   stm32g474xx.h, stm32h563xx.h, stm32u545xx.h, stm32u575xx.h,
   stm32u5a5xx.h, stm32wb55xx.h, stm32wba52xx.h).
3. Descriptors: `stm32l552` TIM1/TIM8 and `stm32f072` TIM1 had no
   `advanced: true`, so EGR.BG and EGR.COMG set no flag.

## Evidence
- `fixed.log`: guard `stm32_advanced_timer_irq_lines` 4/4, default and
  `event-scheduler`.
- `negative-control-old-descriptors.log`: same guard on the old
  descriptors, new model: 81 failing rows.
- Model negative control: the old `0x1F` mask fails 2 of the 4 new timer
  unit tests.
