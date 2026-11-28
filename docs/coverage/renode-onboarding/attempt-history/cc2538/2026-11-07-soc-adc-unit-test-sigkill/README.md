# CC2538 SOC_ADC unit-test build attempt — 2026-11-07

- Command: `CARGO_TARGET_DIR=/dev/shm/renode-cc2538-soc-adc-test CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo +1.95.0 test -p labwired-core --lib cc2538::soc_adc`
- Stage: compiling the `labwired-core` library test binary.
- Observed result: `sccache: Compile terminated by signal 9`; rustc exited before the test binary ran.
- Verdict: **inconclusive**. No SOC_ADC unit test passed or failed. Concurrent builds were active on the shared host; resource pressure is plausible but the SIGKILL cause was not confirmed.
- The separate exact-import CC2538 onboarding smoke completed 4,000,000 instructions and passed all 3 assertions; see `../../../metrics/cc2538/` for its committed logs and result.
