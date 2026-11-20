# Imported ATmega128RFA1 onboarding smoke

This profile measures AVR8 execution and GPIOA/GPIOB against the imported 128 KiB flash
/ 16 KiB SRAM geometry (SRAM at `0x0200..0x41FF`). The LabWired AVR model mirrors
this part's ports A-G at the ATmega1281 addresses (PINA..PORTG at
0x20..0x34) and implements write-one-to-toggle PINx, including SBI on PINx; the
chip maps `porta` and `portb`. Ports C-G are kept by the CPU but no window is
mapped for them. The fixture is
[`tests/fixtures/avr/attiny-portb-io-smoke.c`](../../tests/fixtures/avr/attiny-portb-io-smoke.c)
built with `-mmcu=atmega128rfa1 -DSCRATCH_LEN=48` by
[`scripts/build_attiny_portb_io_smoke.sh`](../../scripts/build_attiny_portb_io_smoke.sh).
It checks DDRB readback, SBI on PORTB keeping PB3 with PINB0 following the
latch, CBI clearing PB3 only, a PINB write toggling PB0 low, SBI on PINB
toggling PB0 only, a 48-byte SRAM block readback, and port A latch/readback
with a PINA toggle (bit 0x40). It stores the check bits at `0x0230` and then
toggles PB0 through PINB in a loop. The smoke asserts the result byte 0x7F, at
least two PB0 edges and the bounded `max_steps` stop after 100,000
instructions.

Not claimed: the 2.4 GHz transceiver, USARTs, SPI, TWI, timers, ADC, analog
comparator, pull-ups, interrupts, EEPROM and the watchdog. The run assumes an ideal power
rail. Result JSON, JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-atmega128rfa1/`](../coverage/renode-onboarding/metrics/simavr-atmega128rfa1/).
This smoke demonstrates the LabWired model only; it was not checked against a
Part Knowledge record. This is local CLI smoke evidence against the imported
manifest, not full ATmega128RFA1 coverage or hardware confirmation.
