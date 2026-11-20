# Superseded lazy FP smoke build attempt

The second target-only Round 531 run did not reach simulation. Although the same
`vldmia r0!, {s0-s15}` instruction builds in an ordinary Rust `asm!` block, the
naked handler's `naked_asm!` block rejected it with
`instruction requires: fp registers`. The raw metric and build logs are kept
here; the generated scoreboard recorded STM32H743 as a build failure until the
corrected target-only rerun.

The handler now calls a small Rust helper containing the FP instruction. This
attempt is retained as build evidence and is not a platform run.
