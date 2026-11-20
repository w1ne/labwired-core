# Negative control: EFM32PG12 TIMERs on the Series-0 register map (discriminating)

The round-389 firmware (`firmware-efm32pg12-smoke`, checks in
`firmware-efm32jg1-smoke/src/s1.rs`) ran against a copy of
`configs/chips/onboarding/efm32pg12.yaml` (`efm32pg12-negctl-chip.yaml`) whose
TIMER0/TIMER1 are `efm32s0_timer` instead of `efm32s1_timer`. Same bases
(0x40018000/0x40018400), same IRQs, same everything else; the system file
differed only in its `chip:` path. Run with the same `target/debug/labwired`
binary that produced the passing metrics.

Result: FAIL 3/5. The six GPIO lines print, then nothing: on the Series-0 map
offset 0x0C is IEN, not IF, so the `tim0 up` check's poll for IF.OF reads 0
until the step budget runs out (`expected_stop_reason: max_steps` still
holds; the ordered UART lines and the summary word 0xEF3101FF do not). The
Series-1 descriptor passes 5/5.
