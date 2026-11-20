# Negative control: onboarding nRF52840 descriptor without SPIM3

Same firmware and fixed engine. Chip and system descriptors from `HEAD`
before this change (no `spi3`, no MAX31855 fixture). With the new system file
the CLI refuses the fixture: `connection 'spi3' is not a SPI peripheral`.

Result with the old system file: FAIL 4/7. The first SPIM3 register write
faults: `HardFault escalated from a precise BusFault: data access at
0x4002_F508` (PSEL.SCK). UART stops after `pwm irq lines ... ok`; RESULT reads
0xa5a5a5a5.
