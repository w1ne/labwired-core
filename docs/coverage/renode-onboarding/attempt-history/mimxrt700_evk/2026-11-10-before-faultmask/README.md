# MIMXRT700-EVK before FAULTMASK smoke

This is the complete target evidence immediately before adding the FAULTMASK
case, preserved before rerunning `mimxrt700_evk`.

- Exact imported system: `configs/systems/onboarding/mimxrt700_evk.yaml`.
- Previous result: 33/33 smoke assertions passed; 20,000,000 instructions to
  `max_steps`; firmware pass bitmap `0x7fff`.
- The current scoreboard at this baseline was 245 descriptors, 226 booted,
  226 booted and smoke-passing, 19 `not_run`, and 0 measured failures.

The full prior logs, result JSON, JUnit, snapshots, UART and metrics are in
`metrics/`.
