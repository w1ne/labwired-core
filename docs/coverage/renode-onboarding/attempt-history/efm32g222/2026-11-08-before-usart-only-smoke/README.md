# Before — EFM32G222 USART0-only smoke

State before round 384: the descriptor claimed only USART0 (flash at 0x08000000, where Renode and silicon have 0x0)
and the smoke asserted one banner line. Kept for comparison with the
GPIO/TIMER smoke (`examples/renode-onboarding/efm32g222-smoke.yaml`).
