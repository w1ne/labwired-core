# Negative control: USART2 without `irq: 33` (2026-11-16)

Chip descriptor = this round's `configs/chips/onboarding/focaltech_ft9001.yaml`
with only the `irq: 33` line on `usart2` removed. Same system yaml, smoke
script, debug CLI (built 2026-11-16 05:25 UTC) and firmware
(SHA-256 c4265b7e3d23e4b43b6e0bbefc9b33fee5dfa5f8c3b2cd39ac1f44a8197c2bfa).

Result: FAIL 2/3 (uart_ordered). The model still computes the level, but no
NVIC line is wired, so NVIC 33 never pends and the handler never runs:

    irq33 idle pend=00 txfie pend=00 off pend=00 rxfie empty pend=00
    irq33 rx pend=00 hits=00 byte=^@ after pend=00 fsr=64
