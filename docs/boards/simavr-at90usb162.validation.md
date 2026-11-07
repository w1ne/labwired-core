# Imported AT90USB162 onboarding smoke

This profile measures bounded AVR8 instruction execution and SRAM access
against the imported 16 KiB flash / 512 B SRAM geometry, with SRAM at
`0x0100..0x02FF`. The fixture increments a volatile SRAM counter, and the smoke
asserts the bounded `max_steps` stop after 100,000 instructions. It makes no
peripheral or board-I/O claims: the USB device controller, timers, USART1, SPI, analog comparator, GPIO ports, EEPROM and the watchdog are not modelled for this profile.

The run assumes an ideal power rail; power-on and brown-out behavior are not
measured. Build the fixture with
[`scripts/build_classic_mega_cpu_smoke.sh`](../../scripts/build_classic_mega_cpu_smoke.sh).
Result JSON, JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-at90usb162/`](../coverage/renode-onboarding/metrics/simavr-at90usb162/).
This is local CLI smoke evidence against the imported manifest, not full
AT90USB162 coverage or hardware confirmation.
