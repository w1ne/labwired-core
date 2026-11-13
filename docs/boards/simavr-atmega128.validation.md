# Imported ATmega128 onboarding smoke

This profile measures AVR8 execution, GPIOA and polled USART0 TX against the
imported 128 KiB flash / 4 KiB SRAM geometry (SRAM from `0x0100`). The
ATmega128 keeps the classic I/O file (PORTA at 0x3B, read-only PINx) and adds
extended I/O at 0x60..0xFF; the AVR core decodes that whole 0x20..0xFF range
with the classic layout rather than the ATmega328P map (where 0x3B is PCIFR).
The chip maps only `porta` (PINA/DDRA/PORTA at 0x39..0x3B) and the CPU's USART
host window, onto which the core translates UBRR0L/UCSR0B/UCSR0A/UDR0 at
0x29..0x2C. The fixture is
[`tests/fixtures/avr/atmega128-io-smoke.c`](../../tests/fixtures/avr/atmega128-io-smoke.c)
built with `-mmcu=atmega128`. It runs seven checks: UBRR0L and UCSR0B readback,
UDRE0 set, SBI on PORTA keeps PA7 and PINA0 follows the latch, CBI clears PA7
only, a PINA write does not toggle (PINx is read-only on this part), and a
160-byte SRAM block at 0x122..0x1C1 reads back. It stores the check bits at
0x01C2, prints `M128 IO bits=7F` and toggles PA0 in a loop. The smoke asserts
the UART text, the result byte 0x7F, at least two PA0 edges and the bounded
`max_steps` stop after 100,000 instructions.

Negative control: the same firmware and script against the previous CPU-only
chip descriptor (`peripherals: []`) fails 0/4 (memory violation at 0x10039).

Not claimed: ports B-G (the core mirrors ports A-E, but only `porta` is mapped
and asserted), UBRR0H/UCSR0C, USART1, timers, ADC, SPI, TWI, interrupts,
EEPROM and the ATmega103 compatibility fuse. The run assumes an ideal power
rail. Result JSON, JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-atmega128/`](../coverage/renode-onboarding/metrics/simavr-atmega128/).
This is local CLI smoke evidence against the imported manifest, not full
ATmega128 coverage or hardware confirmation.
