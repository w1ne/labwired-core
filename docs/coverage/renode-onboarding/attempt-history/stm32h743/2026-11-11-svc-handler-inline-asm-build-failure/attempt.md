# STM32H743 SVC handler CPACR attempt 01

- Result: firmware build failed before a simulation run. LLVM rejected the inline VFP instructions in `naked_asm!` with `instruction requires: fp registers`.
- The failed `onboarding-metrics.json` records `failure_stage: build_firmware` and `run_attempted: false`. This attempt provides no boot evidence.
- The runner left an older `simulation-output` directory in place, so it was removed from this archive to avoid associating the prior run with this build failure.
- Fix: moved the VFP round trip into a normal Rust helper, called by the naked SVC handler. The target-only retry passed and is recorded under `metrics/stm32h743/`.
