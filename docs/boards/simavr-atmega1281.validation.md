# Imported ATmega1281 onboarding smoke

This profile measures AVR8 execution, GPIOA and polled USART0 against the
imported 128 KiB flash / 8 KiB SRAM geometry. The chip maps only `porta`
(PINA/DDRA/PORTA at 0x20..0x22) and `usart0` (0xC0..0xC6). The fixture
[`tests/fixtures/avr/mega164-io-smoke.c`](../../tests/fixtures/avr/mega164-io-smoke.c),
built with `-mmcu=atmega1281 -DPART_TAG='"1281"'`, runs seven checks: UBRR0L and
UCSR0B readback, UDRE0 set, DDRA readback, PINA0 follows the latch, and PINA
write-one-to-toggle low and high again. It stores the check bits at 0x0224,
prints `M1281 IO bits=7F` on USART0, and toggles PA0 in a loop. The smoke
asserts the UART text, the result byte 0x7F, at least two PA0 edges and the
bounded `max_steps` stop after 100,000 instructions.

Not claimed: other ports, timers, ADC, SPI, TWI, USART1+, interrupts and
EEPROM. The run assumes an ideal power rail. Result JSON, JUnit assertions,
simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-atmega1281/`](../coverage/renode-onboarding/metrics/simavr-atmega1281/).
This is local CLI smoke evidence against the imported manifest, not full
ATmega1281 coverage or hardware confirmation.
