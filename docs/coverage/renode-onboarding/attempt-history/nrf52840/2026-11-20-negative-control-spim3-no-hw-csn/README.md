# Negative control: SPIM3 hardware CSN not honoured by the fast SPIM model

Same firmware (`firmware-nrf52840-renode-smoke` with `check_spim3_hw_csn`),
same chip and system descriptors (SPIM3 at 0x4002F000, MAX31855 fixture with
`cs_pin: P0.20`). In `crates/core/src/peripherals/spi.rs` the nRF52 SPIM3 arm
of `do_nrf52_easydma` was disabled (`if false && self.nrf52_hw_select`), which
is the pre-fix behaviour: PSEL.CSN selected devices on the nRF54L map only.
CLI rebuilt.

Result: FAIL 5/7. UART:
`spim3 csn max31855=00000000/00000000 n=8 gpio=1 irq47=1 FAIL`,
`NRF52840 FAIL summary=0ffff` (bit 16 clear). The transfer completes and
pends line 47, but the fixture is never selected (P0.20's GPIO latch is high),
so MISO is 0. With the fix: `max31855=01901600/01901600`.

A third state was seen during development but not kept as an artifact: with
the CSN fix and without the EasyDMA `on_event` IRQ pend, the same line read
`irq47=0 FAIL`.
