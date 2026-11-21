# Negative control: STM32F103 I2C1 without EV/ER IRQ lines (2026-11-20)

Firmware: `firmware-f103-i2c-demo` with the I2C1 EV/ER IRQ check. Descriptor:
`configs/chips/onboarding/stm32f103.yaml` from `origin/main` before this
change (no `irq` and no `irq_error` on `i2c1`).

Result: `I2C1 IRQ EV=00000000 ER=00000000 FAIL`; the smoke fails at
assertion 41. With `irq: 31` and `irq_error: 32` declared the same firmware
reads `EV=00000001 ER=00000002 REL=1 ok` (`metrics/stm32f103`).
