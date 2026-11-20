# STM32F103 channel IRQ run: CLI build stalled

The target-only onboarding pipeline selected the imported `stm32f103` descriptor
and rebuilt `labwired-cli`, but it never reached firmware build or simulator
execution. The build remained blocked in the shared `sccache` wrapper while
compiling `labwired-config` for 269 seconds. The `sccache --show-stats` command
also timed out. I stopped only this run's recorded `cargo` and `sccache` PIDs.

The raw pipeline measurement and CLI build log are preserved beside this note.
This is a host build interruption, not a measured STM32F103 simulation failure;
no boot or smoke result was produced by this attempt.
