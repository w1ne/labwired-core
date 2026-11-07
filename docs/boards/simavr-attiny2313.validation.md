# Imported ATtiny2313 onboarding smoke

This profile measures AVR8 execution, GPIOB and polled USART TX against the
imported 2 KiB flash / 128 B SRAM geometry (SRAM from `0x0060`). ATtiny2313 has the classic 64-byte
I/O file: the chip maps only `portb` (PINB/DDRB/PORTB at 0x36..0x38) and the
CPU's USART host window, onto which the core translates UBRRL/UCSRB/UCSRA/UDR
at 0x29..0x2C. Unlike the ATmega8/16/32/128, PINx on this part is
write-one-to-toggle. The fixture is
[`tests/fixtures/avr/attiny2313-io-smoke.c`](../../tests/fixtures/avr/attiny2313-io-smoke.c)
built with `-mmcu=attiny2313 -DPART_TAG='"2313"'`. It runs seven checks: UBRRL
and UCSRB readback, UDRE set, SBI on PORTB keeps PB7 and PINB0 follows the
latch, CBI clears PB7 only, a PINB write toggles PB0 low, and a 48-byte SRAM
block at 0x84..0xB3 reads back. It stores the check bits at 0x00B4, prints
`T2313 IO bits=7F` and toggles PB0 through PINB in a loop. The smoke asserts
the UART text, the result byte 0x7F, at least two PB0 edges and the bounded
`max_steps` stop after 100,000 instructions.

Negative control: the same firmware and script against the previous CPU-only
chip descriptor (`peripherals: []`) fails 0/4 (memory violation at 0x10036).

Not claimed: ports A/D, UBRRH/UCSRC, timers, USI, analog comparator,
interrupts, EEPROM and the watchdog. The run assumes an ideal power rail.
Result JSON, JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-attiny2313/`](../coverage/renode-onboarding/metrics/simavr-attiny2313/).
This is local CLI smoke evidence against the imported manifest, not full
ATtiny2313 coverage or hardware confirmation.
