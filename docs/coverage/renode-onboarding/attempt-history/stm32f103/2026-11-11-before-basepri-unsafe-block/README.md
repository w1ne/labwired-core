# STM32F103 BASEPRI smoke build attempt

- Target: `stm32f103`
- Started: 2026-11-11 14:55:39 UTC
- Stage: firmware build
- Result: failed before simulation; no firmware instruction ran
- Cause: the new safe exception handlers called inline assembly outside an
  `unsafe` block (`E0133`)
- Evidence: `logs/build_firmware.log` and `onboarding-metrics.json`

The handlers now place their inline assembly in explicit unsafe blocks. The
corrected target run is recorded under `metrics/stm32f103/`.
