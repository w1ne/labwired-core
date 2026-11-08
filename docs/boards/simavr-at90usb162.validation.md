# Imported AT90USB162 onboarding smoke

This profile measures AVR8 execution and GPIOB against the imported 16 KiB flash
/ 512 B SRAM geometry (SRAM at `0x0100..0x02FF`). The LabWired AVR model mirrors
this part's ports at the ATmega328P addresses (PINB/DDRB/PORTB at 0x23..0x25)
and implements write-one-to-toggle PINx, including SBI on PINx; the chip maps
`portb` only. Ports C and D are kept by the CPU but no window is mapped for
them. The fixture is
[`tests/fixtures/avr/attiny-portb-io-smoke.c`](../../tests/fixtures/avr/attiny-portb-io-smoke.c)
built with `-mmcu=at90usb162 -DSCRATCH_LEN=48` by
[`scripts/build_attiny_portb_io_smoke.sh`](../../scripts/build_attiny_portb_io_smoke.sh).
It checks DDRB readback, SBI on PORTB keeping PB3 with PINB0 following the
latch, CBI clearing PB3 only, a PINB write toggling PB0 low, SBI on PINB
toggling PB0 only, and a 48-byte SRAM block readback. The part has no port A, so
the port-A check (bit 0x40) is compiled out. It stores the check bits at
`0x0130` and then toggles PB0 through PINB in a loop. The smoke asserts the
result byte 0x3F, at least two PB0 edges and the bounded `max_steps` stop after
100,000 instructions.

Not claimed: the USB device controller, USART1, SPI, timers, analog comparator,
pull-ups, interrupts, EEPROM and the watchdog. The run assumes an ideal power
rail. Result JSON, JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-at90usb162/`](../coverage/renode-onboarding/metrics/simavr-at90usb162/).
This smoke demonstrates the LabWired model only; it was not checked against a
Part Knowledge record. This is local CLI smoke evidence against the imported
manifest, not full AT90USB162 coverage or hardware confirmation.
