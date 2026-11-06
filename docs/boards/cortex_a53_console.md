# Imported Cortex-A53 console profile

## Measured scope

The imported profile is narrowed to one AArch64 Cortex-A53 processing element
and its 128 MiB DDR window at `0x40000000`. The generic timer, PL011, virtio
console, flash, reset flow, and full board behavior are not modeled or tested.
The whole-platform descriptor remains unverified.

## Smoke evidence

The exact imported system runs the committed AArch64 integer-loop ELF directly
from DDR for 100,000 instructions and asserts the `max_steps` stop. The fixture
performs no MMIO. This is local LabWired CLI evidence for the bounded AArch64
execution and memory profile. It is not Renode execution, console or peripheral
validation, reset-vector boot, hosted `labwired_verify`, or hardware
confirmation.

Evidence: `docs/coverage/renode-onboarding/metrics/cortex_a53_console/` contains
the metric, JUnit assertion, simulator result, snapshot, and run log.
