# Imported ATmega16 onboarding smoke

This profile measures bounded AVR8 instruction execution and SRAM access
against the imported 16 KiB flash / 1 KiB SRAM geometry, with SRAM beginning at
`0x0060`. The fixture increments a volatile SRAM counter, and the smoke asserts
the bounded `max_steps` stop after 100,000 instructions. It makes no peripheral
or board-I/O claims.

The run assumes an ideal power rail; power-on and brown-out behavior are not
measured. Build the fixture with
[`scripts/build_atmega16_cpu_smoke.sh`](../../scripts/build_atmega16_cpu_smoke.sh).
Result JSON, JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-atmega16/`](../coverage/renode-onboarding/metrics/simavr-atmega16/).
This is local CLI smoke evidence against the imported manifest, not full
ATmega16 coverage or hardware confirmation.
