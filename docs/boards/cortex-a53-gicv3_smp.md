# Cortex-A53 GICv3 SMP imported profile

## Measured scope

The imported four-core profile is narrowed to one AArch64 Cortex-A53 processing
element and the 2 GiB DDR window at `0x40000000`. The profile omits multicore
execution, GICv3, generic timers, PL011 UART, and PL031 RTC. Its whole-platform
descriptor remains unverified.

## Smoke evidence

The exact imported system ran the committed AArch64 integer-loop ELF for
100,000 instructions and passed its `max_steps` assertion. It recorded zero
peripheral accesses. This is local LabWired CLI evidence for the bounded
single-PE CPU-and-DDR profile. It is not a hosted `labwired_verify` verdict,
Renode execution, reset-vector boot, SMP/GIC/peripheral validation, or hardware
confirmation.

Evidence is under
`docs/coverage/renode-onboarding/metrics/cortex-a53-gicv3_smp/`, including the
metric, JUnit assertion, simulator result, snapshot, and run log.
