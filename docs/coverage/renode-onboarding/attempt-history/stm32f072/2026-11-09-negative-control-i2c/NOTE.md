Negative control (2026-11-09), final firmware and engine: I2C1 with
`profile: stm32f1` (the legacy CR1/SR1/DR I2C that configs/chips/stm32f072.yaml
declares for the F072) instead of the F0's "v2" TIMINGR/ISR I2C. FAIL 6/8:
the run never gets past the I2C check (the v2 ISR/TXDR sequence finds no
TXIS, every later line is missing) within the 4M-step budget.
