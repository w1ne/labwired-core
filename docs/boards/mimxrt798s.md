# Renode onboarding evidence: mimxrt798s

The old generated report for this descriptor ran an incompatible Cortex-M3
fixture (`uart-ok-thumbv7m.elf`) that faulted at step 2, and listed
peripherals that the chip descriptor does not map. This report replaces that
claim with a smoke run against the imported MIMXRT798S system and its
CPU0/flexcomm0 profile.

## Measured run

- Status: **booted and smoke-passing**.
- Firmware: `firmware-mimxrt798s-smoke`, Cortex-M33, 4,000,000 instructions.
- Stop reason: `max_steps` after all checks completed.
- Assertions: UART banner and ordered register transcript, two SRAM values, and
  the stop reason (5/5 passed).
- UART output checked the LP_FLEXCOMM0 PSELID reset value, UART selection,
  write-once lock, LPUART VERID/PARAM/BAUD/CTRL, and UARTTX interrupt status.
- Evidence: `docs/coverage/renode-onboarding/metrics/mimxrt798s/` (build/run
  logs, UART output, JUnit, result, and snapshot).

The earlier CPU/memory-only measurement is preserved at
`docs/coverage/renode-onboarding/attempt-history/mimxrt798s/2026-11-08-before-lp-flexcomm-smoke/`.
A negative control with `flexcomm0` removed fails 2/5 with a precise BusFault
at BFAR 0x40110FF8:
`docs/coverage/renode-onboarding/attempt-history/mimxrt798s/2026-11-08-flexcomm0-negative-control/`.

## Profile boundary

The imported chip profile includes Cortex-M33 CPU0, the XIP1 flash window at
`0x08000000`, CPU0 non-secure SRAM, and `flexcomm0` at `0x40110000` (Renode
`cpus/mimxrt798s.repl`). The LP_FLEXCOMM wrapper models PSELID/ISTAT and the
LPUART register path used by this smoke. Its configured 80 MHz functional clock
is used for serial timing; the smoke does not validate an external baud rate or
pin routing.

LPI2C and LPSPI transactions, Flexcomm1-13, the secure alias, GPIO/pin mux,
clock/reset controllers, CPU1, the DSPs, XSPI2, and other Renode peripherals
are not claimed. Simulation assumes an ideal VDD rail; power-on and brown-out
behavior were not exercised. This is repository CLI simulation evidence, not
hardware confirmation or a silicon-wide peripheral claim.
