# Negative control: CH32X035 EXTI on the wrong AFIO/EXTI layouts

The same unmodified `exti_pin_change_isr.elf`, the same system and stimuli,
on `chip.yaml` here, a copy of `configs/chips/ch32x035c8t6-ch32fun-smoke.yaml` with two lines
changed: `afio` `profile: stm32f1` (4-bit EXTICR fields) and `exti`
`profile: ch32v20x` (the V20x per-line vectors). It fails 7/9: AFIO_EXTICR1
reads back 0 (the F1 mask drops bit 15, so line 7 stays on port A), PC7's
edges never reach EXTI, and PC14 never moves.
