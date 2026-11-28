# nrf52840 negative control: ECB peripheral removed (2026-11-20)

Purpose: show that the new `ecb fips197` line depends on the ECB model, not on
the firmware alone.

Method: a `/tmp` copy of `configs/chips/onboarding/nrf52840.yaml` with the
`ecb` peripheral (0x4000E000) deleted, a system file pointing at it, and the
committed `nrf52840-smoke.yaml` with absolute paths. Same firmware and
`target/debug/labwired` CLI as the passing run.

Result: 3 of 5 assertions fail. The firmware takes a precise BusFault on the
first ECB write (`EVENTS_ENDECB`, 0x4000_E100); UART stops after the `temp`
line; RESULT stays 0xa5a5a5a5. Files: `uart.log`, `junit.xml`, `run.log`.
