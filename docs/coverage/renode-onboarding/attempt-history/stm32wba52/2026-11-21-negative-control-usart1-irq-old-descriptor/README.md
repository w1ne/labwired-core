# Negative control: STM32WBA52 USART1 without its IRQ line (2026-11-21)

Firmware: `firmware-nucleo-wba52cg-demo` with the USART1 TCIE IRQ check.
Descriptor: `configs/chips/onboarding/stm32wba52.yaml` from `origin/main`
before this change (no `irq` on `usart1`).

Result: `usart1 tcie ispr40=00 rel=00 FAIL`; the smoke fails 5/6. With
`irq: 46` declared the same firmware reads `usart1 tcie ispr40=40 rel=00 ok`
(`metrics/stm32wba52`, `metrics/nucleo_wba52cg`).
