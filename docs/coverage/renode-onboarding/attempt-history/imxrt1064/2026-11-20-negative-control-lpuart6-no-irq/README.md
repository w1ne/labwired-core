# Negative control: LPUART6 IRQ line, none

Same firmware (`firmware-imxrt1064-smoke` with `onboarding-adc`), same system
and smoke script as the passing run; only the chip descriptor differs:
the imported descriptor before this fix (`lpuart6` has no `irq`).

Result: `lpuart6 tcie ispr=00 rel=00 FAIL`, smoke status `fail`.

`ispr` is NVIC ISPR bits 20..27 (LPUART1..8) as one byte while CTRL.TCIE is
set and the lines are masked. Silicon and Renode's `imxrt1064.repl` pend
line 25 (`ispr=20`).
