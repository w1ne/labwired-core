# Imported ATmega1280 onboarding smoke

This profile measures AVR8 instruction execution and SRAM access against the
imported 128 KiB flash / 8 KiB SRAM geometry. The fixture increments a volatile
SRAM counter, and the smoke asserts the bounded `max_steps` stop after 100,000
instructions. The run recorded 185,705 cycles, 28,570 memory writes, and zero
peripheral accesses. It makes no peripheral or board-I/O claims.

For this bounded run, the imported target uses the existing ATmega1281 CPU
geometry, which matches the imported ATmega1280 profile's 128 KiB flash and
8 KiB SRAM ranges. This geometry alias does not validate ATmega1280 peripheral
behavior. The run assumes an ideal power rail, so power-on and brown-out
behavior are not measured.

Build the fixture with `scripts/build_atmega1280_cpu_smoke.sh`. Result JSON,
JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-atmega1280/`](../coverage/renode-onboarding/metrics/simavr-atmega1280/).
This is local CLI smoke evidence against the imported manifest, not full
ATmega1280 coverage or hardware confirmation.
