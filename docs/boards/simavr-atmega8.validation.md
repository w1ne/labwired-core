# Imported ATmega8 onboarding smoke

This profile measures AVR8 execution, GPIOB and polled USART TX against the
imported 8 KiB flash / 1 KiB SRAM geometry (SRAM from `0x0060`). ATmega8 has
the classic 64-byte I/O file but no port A: the chip maps only `portb`
(PINB/DDRB/PORTB at 0x36..0x38) and the CPU's USART host window, onto which the
core translates UBRRL/UCSRB/UCSRA/UDR at 0x29..0x2C. The fixture is
[`tests/fixtures/avr/atmega8-io-smoke.c`](../../tests/fixtures/avr/atmega8-io-smoke.c)
built with `-mmcu=atmega8`. It runs seven checks: UBRRL and UCSRB readback,
UDRE set, SBI on PORTB keeps PB7 and PINB0 follows the latch, CBI clears PB7
only, a PINB write does not toggle (PINx is read-only on this part), and a
160-byte SRAM block at 0x80..0x11F reads back. It stores the check bits at
0x0120, prints `M8 IO bits=7F` and toggles PB0 in a loop. The smoke asserts the
UART text, the result byte 0x7F, at least two PB0 edges and the bounded
`max_steps` stop after 100,000 instructions.

Negative control: the same firmware and script against the previous CPU-only
chip descriptor (`peripherals: []`) fails 0/4.

Not claimed: ports C/D, UBRRH/UCSRC, timers, ADC, SPI, TWI, interrupts and
EEPROM. The run assumes an ideal power rail. Result JSON, JUnit assertions,
simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-atmega8/`](../coverage/renode-onboarding/metrics/simavr-atmega8/).
This is local CLI smoke evidence against the imported manifest, not full
ATmega8 coverage or hardware confirmation.
