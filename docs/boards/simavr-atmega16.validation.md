# Imported ATmega16 onboarding smoke

This profile measures AVR8 execution, GPIOA and polled USART TX against the
imported 16 KiB flash / 1 KiB SRAM geometry (SRAM from `0x0060`). ATmega16 has
the classic 64-byte I/O file: the chip maps only `porta` (PINA/DDRA/PORTA at
0x39..0x3B) and the CPU's USART host window, onto which the core translates
UBRRL/UCSRB/UCSRA/UDR at 0x29..0x2C. The fixture
[`tests/fixtures/avr/atmega16-io-smoke.c`](../../tests/fixtures/avr/atmega16-io-smoke.c)
runs seven checks: UBRRL and UCSRB readback, UDRE set, SBI on PORTA keeps PA7
and PINA0 follows the latch, CBI clears PA7 only, a PINA write does not toggle
(PINx is read-only on this part), and a 160-byte SRAM block at 0x82..0x121
reads back. That block covers addresses the ATmega328P map decodes as timer,
ADC, TWI and USART0 registers. It stores the check bits at 0x0122, prints
`M16 IO bits=7F` and toggles PA0 in a loop. The smoke asserts the UART text,
the result byte 0x7F, at least two PA0 edges and the bounded `max_steps` stop
after 100,000 instructions.

Not claimed: ports B-D, UBRRH/UCSRC, timers, ADC, SPI, TWI, interrupts and
EEPROM. The run assumes an ideal power rail. Result JSON, JUnit assertions,
simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-atmega16/`](../coverage/renode-onboarding/metrics/simavr-atmega16/).
This is local CLI smoke evidence against the imported manifest, not full
ATmega16 coverage or hardware confirmation.
