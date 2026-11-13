# Superseded lazy FP smoke build attempt

The target-only Round 531 run at 23:19 UTC did not reach simulation. The
firmware build rejected `vmov.f32 s0, s1` in the naked IRQ handler with
`instruction requires: fp registers`. The raw metric and build logs are kept
here; the generated scoreboard recorded STM32H743 as a build failure until the
corrected target-only rerun.

The first replacement tried `vldmia` directly in the naked handler; that also
failed because the naked assembly block did not enable FP registers. Its raw
failure is in the sibling `2026-11-10-lazy-fp-naked-asm-build-failure/`
directory. The final fixture calls a normal Rust helper containing `vldmia`,
which compiles with the target's FP-register features. This attempt is retained
as build evidence and is not a platform run.
