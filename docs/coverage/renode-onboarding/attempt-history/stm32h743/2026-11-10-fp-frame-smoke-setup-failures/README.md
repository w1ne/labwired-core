# Superseded Cortex-M7 FP smoke setup attempts

These are development attempts from Round 530, not the final platform result. The
runner's per-target working artifacts were replaced by later retries before the
failed attempts were copied, so this note reconstructs their outcome from the
runner output; it is not a substitute for raw simulation evidence. The final
measured result is under `docs/coverage/renode-onboarding/metrics/stm32h743/`.

1. At 22:36 UTC, the firmware build used `thumbv7em-none-eabi`, which does not
   enable the FP register set. The assembler rejected `vldmia`, `vmsr`, `vstmia`,
   and `vmrs` with `instruction requires: fp registers`. No simulation ran.
2. At 22:37 UTC, the build used `thumbv7em-none-eabihf`, but the smoke YAML still
   referenced the old soft-float output path. The simulator booted the prior
   image and reached its 4,000,000-instruction limit; the new FP UART assertion
   failed because that image had no FP test. The metric reported
   `failure_stage: run_smoke` and `captured len=7`.

The mapping and smoke input now both use the hard-float target. The next
one-target run passed the FP frame, FPCAR, S0-S15/FPSCR restoration, and existing
Cortex-M7 smoke assertions. These setup failures are not counted as platform
measurements in the generated scoreboard.
