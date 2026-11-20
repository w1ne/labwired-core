# STM32F072 USART shared IRQ failure before level-line fix

This is the committed exact-target evidence from the 2026-11-13 pre-fix
measurement. The imported STM32F072 system booted and executed 4,000,000
instructions, but the USART3/USART4 shared IRQ 29 smoke failed because TXEIE
did not consistently reach NVIC pending. The failure and its complete logs,
UART output, JUnit result, simulator result, and snapshot are preserved here.

A further target-only reproduction at 06:53 UTC on the same pre-fix code again
failed USART3 (quiet=1, pend=0, released=1). Its worktree metrics were replaced
by the post-fix run; the committed pre-fix evidence above records the same
failure class. The current pass is under
docs/coverage/renode-onboarding/metrics/stm32f072/.
