# Negative control: TWIM address-NACK model before the fix (2026-11-20)

Same firmware and smoke script as the passing nrf52840 run (round 747), built
against the previous `crates/core/src/peripherals/nrf52/twim.rs`. That model
fired `EVENTS_LASTTX` on an address NACK and ran the LASTTX_STOP shortcut.

The TWIM0 check arms LASTTX_STOP, addresses the empty 0x49 and, after
`EVENTS_ERROR`, requires `EVENTS_LASTTX == 0`, `TXD.AMOUNT == 0` and no
`EVENTS_STOPPED` before its own `TASKS_STOP` (nrfx `nrfy_twim_tx_start`:
"triggering of STOP task in case of error has to be done manually").

Result: `twim0 tmp102 cfg=fff0 temp=1900 anack=1 held=0 w1c=1 FAIL`, summary
`2fff` (bit 12 clear). 2 of 7 assertions fail (`uart_ordered`,
`memory_value`). All other blocks pass. With the fixed model the line reads
`held=1 ... ok` and 7/7 pass.
