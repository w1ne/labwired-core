# Negative control: STM32F401 Black Pill TIM1/9/10/11 without IRQ lines (2026-11-21)

Firmware: `firmware-f401cdu6-blackpill-renode-smoke` with the TIM1/9/10/11
NVIC line check (`smoke_tim1_9_10_11_irq_lines`).
Descriptor: `configs/chips/onboarding/stm32f401cdu6-blackpill.yaml` from
`origin/main` before this change (no `irq` on `tim1`, `tim9`, `tim10`,
`tim11`).

Result: `tim1/9/10/11 irq ispr24=00000000 FAIL`; the smoke fails 2/4. With
TIM1 on 25 (CC on 27), TIM9 on 24, TIM10 on 25 and TIM11 on 26 the same
firmware reads `tim1/9/10/11 irq ispr24=00028124 ok` and passes 4/4.
