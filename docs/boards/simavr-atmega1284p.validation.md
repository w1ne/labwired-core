# Imported ATmega1284P onboarding smoke

This profile measures AVR8 instruction execution and SRAM access against the
imported 128 KiB flash / 16 KiB SRAM geometry. The fixture increments a volatile
SRAM counter and the smoke asserts the bounded `max_steps` stop after 100,000
instructions. It performs no peripheral accesses and asserts no board I/O.

The first attempt stopped during C runtime startup at `0x40ff` because the AVR
CPU path had selected ATmega328P SRAM bounds. Added separate ATmega1284P flash
and SRAM geometry, then reran only this imported target. The failed attempt is
preserved in
[`attempt-history`](../coverage/renode-onboarding/attempt-history/simavr-atmega1284p/initial-stack-boundary/).

LabWired Part Knowledge returned `NOT_FOUND` for ATmega1284P. Peripheral
registers and board behavior therefore remain outside this profile; the result
is local CLI evidence for the CPU/SRAM smoke only.

Build the fixture with `scripts/build_atmega1284p_cpu_smoke.sh`. Result JSON,
JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-atmega1284p/`](../coverage/renode-onboarding/metrics/simavr-atmega1284p/).
The final run executed 100,000 instructions / 185,705 cycles, passed its single
smoke assertion, and recorded zero peripheral accesses. It assumes an ideal
power rail, so power-on and brown-out behavior are not measured.
