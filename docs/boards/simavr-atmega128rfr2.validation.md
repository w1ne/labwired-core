# Imported ATmega128RFR2 onboarding smoke

This profile measures bounded AVR8 instruction execution and SRAM access
against the imported 128 KiB flash / 16 KiB SRAM geometry, with SRAM at
`0x0200..0x41FF`. The fixture increments a volatile SRAM counter, and the smoke
asserts the bounded `max_steps` stop after 100,000 instructions. It makes no
peripheral or board-I/O claims: the 2.4 GHz IEEE 802.15.4 transceiver, timers, USART0/1, SPI, TWI, ADC, AES engine, JTAG, GPIO ports, EEPROM and the watchdog are not modelled for this profile.

The run assumes an ideal power rail; power-on and brown-out behavior are not
measured. Build the fixture with
[`scripts/build_classic_mega_cpu_smoke.sh`](../../scripts/build_classic_mega_cpu_smoke.sh).
Result JSON, JUnit assertions, simulator logs, and the snapshot are under
[`docs/coverage/renode-onboarding/metrics/simavr-atmega128rfr2/`](../coverage/renode-onboarding/metrics/simavr-atmega128rfr2/).
This is local CLI smoke evidence against the imported manifest, not full
ATmega128RFR2 coverage or hardware confirmation.
