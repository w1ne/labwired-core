# Imported ATtiny1634 onboarding smoke

This profile measures AVR8 execution and GPIOA and GPIOB against the imported
16 KiB flash / 1 KiB SRAM geometry (SRAM at `0x0100..0x04FF`). The LabWired AVR
model implements write-one-to-toggle PINx for this part; the chip maps `portb`
(PINB/DDRB/PORTB at 0x2B..0x2D) and `porta` (PINA/DDRA/PORTA at 0x2F..0x31).
The PUEx pull-up registers that follow each port (0x2A/0x2E/0x32) are stored
bytes only: pull-up effects are not modelled. Port C registers are kept by the
CPU but no `portc` window is mapped. The fixture is
[`tests/fixtures/avr/attiny-portb-io-smoke.c`](../../tests/fixtures/avr/attiny-portb-io-smoke.c)
built with `-mmcu=attiny1634 -DSCRATCH_LEN=48` by
[`scripts/build_attiny_portb_io_smoke.sh`](../../scripts/build_attiny_portb_io_smoke.sh).
It checks DDRB readback, SBI on PORTB keeping PB3 with PINB0 following the
latch, CBI clearing PB3 only, a PINB write toggling PB0 low, SBI on PINB
toggling PB0 only, and a 48-byte SRAM block readback, and on port A a DDRA/PORTA
readback with PINA7 following the latch and a PINA write toggling PA7 (bit
0x40). It stores the check bits at `0x0130` and then toggles PB0 through PINB in
a loop. The smoke asserts the result byte 0x7F, at least two PB0 edges and the
bounded `max_steps` stop after 100,000 instructions.

Not claimed: USART0/1, TWI, SPI, USI, timers, ADC, analog comparator, the touch
controller, pull-ups, interrupts, EEPROM and the watchdog. The run assumes an
ideal power rail.
Result JSON, JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-attiny1634/`](../coverage/renode-onboarding/metrics/simavr-attiny1634/).
This smoke demonstrates the LabWired model only; it was not checked against a
Part Knowledge record.
This is local CLI smoke evidence against the imported manifest, not full
ATtiny1634 coverage or hardware confirmation.
