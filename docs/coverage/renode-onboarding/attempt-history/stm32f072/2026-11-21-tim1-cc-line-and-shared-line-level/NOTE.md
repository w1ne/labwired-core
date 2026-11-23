# TIM1 capture/compare line + shared-line level reconcile (2026-11-21)

Guard: `crates/core/tests/stm32/stm32_advanced_timer_irq_lines.rs`
(`cargo test -p labwired-core --test stm32 stm32_advanced_timer_irq_lines`).

- `fixed.log` — new descriptors, new bus reconcile: 3/3 pass.
- `negative-control-old-descriptors.log` — HEAD descriptors, new bus
  reconcile: every one of the 31 timer rows fails, and F103 ADC2 does not pend
  line 18. On this row (onboarding/stm32f072) TIM1 CC1G pended line 13, not
  TIM1_CC_IRQn = 14.
- `negative-control-old-bus-reconcile.log` — new descriptors, HEAD
  `bus/tick.rs` + `bus/device_hooks.rs`: L476/L432 TIM1 update never shows on
  line 25, because idle TIM16 (same line) drops the level pend.
- `stm32f042-smoke.log`, `stm32f072-smoke.log` — the imported rows still pass
  6/6 and 8/8 with the new descriptors (CLI built before the bus change).

IRQ numbers: vendor CMSIS device headers from the stm32duino core
(`stm32f072xb.h`: TIM1_BRK_UP_TRG_COM_IRQn = 13, TIM1_CC_IRQn = 14).
