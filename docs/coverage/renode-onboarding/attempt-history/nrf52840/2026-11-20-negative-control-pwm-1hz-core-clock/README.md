# Negative control: 1 Hz core clock for PWM0 (2026-11-20)

Same firmware and smoke script as the passing nrf52840 run. The /tmp copy of
`configs/systems/onboarding/nrf52840.yaml` (`system.yaml` here) adds
`cpu_hz: 1`. PWM0 then converts 400 PWM ticks to 0 CPU cycles, loads no
waveform and never drives P0.15. This is the state the engine was in before
`Nrf52Pwm::attach_cpu_hz` kept its 64 MHz default for an undeclared clock
(the onboarding chip declares no `cpu_hz`, so the bus passed 0, stored as
1 Hz).

Result: `pwm0 p0.15 seqend=1 duty=0 rises=0 stop=1 idle=0 FAIL`, samples
`high=0000/1f40 rises=0000`, summary word `1fff` (bit 13 clear). 2 of 5
assertions fail (`uart_ordered`, `memory_value`). All other blocks pass, so
the PWM0 check fails on a flat pad and on nothing else.
