# Imported ATtiny2313 onboarding smoke

This profile measures bounded AVR8 instruction execution and SRAM access
against the imported 2 KiB flash / 128 B SRAM geometry, with SRAM at
`0x0060..0x00DF`. The fixture increments a volatile SRAM counter, and the smoke
asserts the bounded `max_steps` stop after 100,000 instructions. It makes no
peripheral or board-I/O claims: timers, USART, USI, analog comparator, GPIO ports and the watchdog are not modelled for this profile.

The run assumes an ideal power rail; power-on and brown-out behavior are not
measured. Build the fixture with
[`scripts/build_attiny_x5_cpu_smoke.sh`](../../scripts/build_attiny_x5_cpu_smoke.sh).
Result JSON, JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-attiny2313/`](../coverage/renode-onboarding/metrics/simavr-attiny2313/).
This is local CLI smoke evidence against the imported manifest, not full
ATtiny2313 coverage or hardware confirmation.
