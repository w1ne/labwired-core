# Negative control: TMP102 fixture removed (2026-11-20)

Same firmware and smoke script as the passing nrf52840 run, with the
`tmp102_twi0` external device removed from a /tmp copy of
`configs/systems/onboarding/nrf52840.yaml` (`system.yaml` here).

Result: TWIM0 sees no slave at 0x48. Every transfer takes ANACK and the RX
buffer fills with 0xFF, so the line reads `cfg=ffff temp=ffff ... FAIL` and the
summary word is `0fff` (bit 12 clear). 2 of 5 assertions fail (`uart_ordered`,
`memory_value`). All other blocks pass, so the TWIM0 check depends on the
attached device and nothing else.
