# Imported ATtiny24 onboarding smoke

This profile measures AVR8 execution and GPIOA and GPIOB against the imported
2 KiB flash / 128 B SRAM geometry (SRAM from `0x0060`). The LabWired AVR model implements the classic 64-byte I/O file and
write-one-to-toggle PINx behavior in this profile; the chip maps
`portb` (PINB/DDRB/PORTB at 0x36..0x38) and `porta` (PINA/DDRA/PORTA at 0x39..0x3B). This imported profile omits USART, so the result is
read from SRAM and from PB0 edges. The fixture is
[`tests/fixtures/avr/attiny-portb-io-smoke.c`](../../tests/fixtures/avr/attiny-portb-io-smoke.c)
built with `-mmcu=attiny24 -DSCRATCH_LEN=48` by
[`scripts/build_attiny_portb_io_smoke.sh`](../../scripts/build_attiny_portb_io_smoke.sh).
It checks DDRB readback, SBI on PORTB keeping PB3 with PINB0 following the
latch, CBI clearing PB3 only, a PINB write toggling PB0 low, SBI on PINB
toggling PB0 only, and a 48-byte SRAM block readback, and on port A a DDRA/PORTA readback with PINA7 following the latch and a PINA write toggling PA7 (bit 0x40). It stores the
check bits at `0x0090` and then toggles PB0 through PINB in a loop. The smoke
asserts the result byte 0x7F, at least two PB0 edges and the bounded
`max_steps` stop after 100,000 instructions.

Not claimed: timers, USI, ADC, analog comparator, interrupts, EEPROM and the watchdog. The run assumes an ideal power rail.
Result JSON, JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-attiny24/`](../coverage/renode-onboarding/metrics/simavr-attiny24/).
Part Knowledge has no public verified record for ATtiny24; this smoke demonstrates
the LabWired model only.
This is local CLI smoke evidence against the imported manifest, not full
ATtiny24 coverage or hardware confirmation.
