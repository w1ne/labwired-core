# Negative control: LPUART6 IRQ line, 21

Same firmware (`firmware-imxrt1064-smoke` with `onboarding-adc`), same system
and smoke script as the passing run; only the chip descriptor differs:
`lpuart6` on `irq: 21` (the value `configs/chips/imxrt1064.yaml` carried; LPUART2's line).

Result: `lpuart6 tcie ispr=02 rel=00 FAIL`, smoke status `fail`.

`ispr` is NVIC ISPR bits 20..27 (LPUART1..8) as one byte while CTRL.TCIE is
set and the lines are masked. Silicon and Renode's `imxrt1064.repl` pend
line 25 (`ispr=20`).
