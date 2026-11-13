# Renode onboarding evidence: mimxrt700_evk

The old generated report for this descriptor used an incompatible Cortex-M7
fixture and faulted before useful firmware execution. This report replaces that
claim with a smoke run against the imported MIMXRT700-EVK system and its
CPU0/flexcomm0 profile.

## Measured run

- Status: **booted and smoke-passing**.
- Firmware: `firmware-mimxrt700-evk-smoke`, Cortex-M33, 4,000,000 instructions.
- Stop reason: `max_steps` after all checks completed.
- Assertions: UART banner and ordered register transcript, two SRAM values, and
  the stop reason (5/5 passed).
- UART output checked the LP_FLEXCOMM0 PSELID reset value, UART selection,
  write-once lock, LPUART VERID/PARAM/BAUD/CTRL, and UARTTX interrupt status.
- Evidence: `docs/coverage/renode-onboarding/metrics/mimxrt700_evk/` (build/run
  logs, UART output, JUnit, result, and snapshot).

The first LP_FLEXCOMM smoke attempt booted but failed two assertions because it
read ISTAT after queueing the banner, while TX was busy. That run is preserved
at `docs/coverage/renode-onboarding/attempt-history/mimxrt700_evk/2026-11-08-first-lp-flexcomm-run-failed-assertion/`;
the firmware now samples the idle-TX status before sending the banner.

## Profile boundary

The imported chip profile includes Cortex-M33 CPU0, the XIP0 flash window,
CPU0 non-secure SRAM, and `flexcomm0` at `0x40110000`. The LP_FLEXCOMM wrapper
models PSELID/ISTAT and the LPUART register path used by this smoke. Its
configured 80 MHz functional clock is used for serial timing; the smoke does
not validate an external baud rate or pin routing.

LPI2C and LPSPI transactions, other Flexcomm instances, the secure alias,
GPIO/pin mux, clock/reset controllers, CPU1, the DSPs, and other generated
peripheral entries are not claimed. Simulation assumes an ideal VDD rail;
power-on and brown-out behavior were not exercised. This is repository CLI
simulation evidence, not hardware confirmation or a silicon-wide peripheral
claim.
