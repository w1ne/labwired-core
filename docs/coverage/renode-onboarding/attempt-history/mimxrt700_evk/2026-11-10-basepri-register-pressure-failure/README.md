# BASEPRI smoke attempt 1

The `mimxrt700_evk` target-only pipeline reached firmware build and stopped
before simulation. Rust reported `inline assembly requires more registers than
available` for the first combined BASEPRI setup and IRQ polling block. The
metrics and compiler output are preserved here; the runner recorded one
failed target before the successful replay restored the current scoreboard.

The assembly was split into setup, interrupt-polling, and state-readback blocks
to reduce register pressure. The follow-up attempt is recorded in the current
target metrics if it completes.
