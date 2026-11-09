Negative control (2026-11-09), final firmware and engine: the chip profile
with `profile: stm32f0` removed from `syscfg`, so it falls back to the
read-zero stub. FAIL 6/8: `exti pads ... pf1=0 pc13=0`, summary 0x072FDFFF.
The pad-routing check depends on the F0 SYSCFG model.
