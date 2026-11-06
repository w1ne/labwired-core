# Imported ATmega1281 onboarding smoke

This profile measures AVR8 instruction execution and SRAM access against the
imported 128 KiB flash / 8 KiB SRAM geometry. The fixture increments a volatile
SRAM counter, and the smoke asserts the bounded `max_steps` stop after 100,000
instructions. It makes no peripheral or board-I/O claims.

The run assumes an ideal power rail, so power-on and brown-out behavior are not
measured. Build the fixture with
[`scripts/build_atmega1281_cpu_smoke.sh`](../../scripts/build_atmega1281_cpu_smoke.sh).
Result JSON, JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-atmega1281/`](../coverage/renode-onboarding/metrics/simavr-atmega1281/).
This is local CLI smoke evidence against the imported manifest, not full
ATmega1281 coverage or hardware confirmation.
