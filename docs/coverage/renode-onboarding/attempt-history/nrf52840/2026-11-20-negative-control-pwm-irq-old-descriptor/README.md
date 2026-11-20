# Negative control: PWM1/PWM2 IRQ numbers from the pre-fix onboarding descriptor

Same firmware and PWM model as the passing run (`firmware-nrf52840-renode-smoke`
with `check_pwm_irq_lines`, PWM level IRQ line). Chip descriptor
`configs/chips/onboarding/nrf52840.yaml` from `HEAD` before this change
(PDM irq 31, PWM1 irq 29, PWM2 irq 30), with the CLI rebuilt.

Result: FAIL 5/7. UART: `pwm irq lines 28..35 pwm1=02 pwm2=04 FAIL`,
`NRF52840 FAIL summary=7fff` (bit 15 clear). Bit 0 of the window is NVIC
line 28, so PWM1 pended line 29 (PDM's vector) and PWM2 pended line 30 (no
interrupt on silicon). With the fixed descriptor: `pwm1=20 pwm2=40` (lines 33
and 34, nrf52840.svd).
