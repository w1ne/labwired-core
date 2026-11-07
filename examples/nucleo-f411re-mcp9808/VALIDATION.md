# Validation record

The committed onboarding smoke builds the Arduino firmware, boots it on the
NUCLEO-F411RE system descriptor, and requires the USART2 output
`MCP9808_OK id=54 dev=400 temp_x16=...`. Firmware prints this marker only after
reading both MCP9808 identity registers and an ambient-temperature sample
within 24–26 °C.

Recorded 2026-11-05: the local onboarding CLI returned `PASS 1/1 checks` after
180,428 steps. The committed result records the `mcp9808` device on `i2c1` at
`0x18`, `MANUF_ID=0x0054`, `DEV_ID=0x0400`, and `TA=400` (25 °C at the default
input).

Evidence is stored under `evidence/`:

- `logs/build_cli.log` — local CLI build
- `logs/build_firmware.log` — PlatformIO firmware build
- `logs/run_smoke.log` — assertion-run output
- `simulation-output/result.json` — simulator status, assertions, and I2C
  peripheral inventory
- `vendor-status.md` — live status and purchase check

This is simulation-side evidence. It does not establish hardware-confirmed
behavior.
