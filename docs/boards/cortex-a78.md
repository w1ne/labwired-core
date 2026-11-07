# Cortex-A78 imported profile

## Measured scope

The imported Cortex-A78 descriptor is narrowed to the engine's shared AArch64
integer execution path and the imported 2 GiB DDR window at `0x40000000`.
The generic timer, PL011, PL031, boot flash, Cortex-A78-specific features,
multicore behavior, and reset-vector boot are not modeled or tested. The
whole-platform descriptor remains `verified: false`.

## Smoke evidence

The exact imported system runs the committed AArch64 integer-loop ELF directly
from DDR for 100,000 instructions and asserts the `max_steps` stop. The fixture
performs no MMIO. This is local LabWired CLI evidence for the bounded AArch64
execution and memory profile. It is not an exact Cortex-A78 microarchitecture
validation, a hosted `labwired_verify` verdict, Renode execution, peripheral
validation, reset-vector boot, or hardware confirmation.

Evidence: `docs/coverage/renode-onboarding/metrics/cortex-a78/` contains the
metric, JUnit assertion, simulator result, snapshot, and run log.
