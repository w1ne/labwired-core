# Cortex-R52 SMP validation status

## Current result

This imported descriptor is present but **not run**. The inventory has no
compatible Cortex-R52 CPU model or firmware smoke, so it records `not_run` with
`run_attempted: false`. The profile now identifies `core: cortex-r52` and keeps
only its imported memory windows; no CPU execution or peripheral behavior is
claimed. The catalog entry is marked `verified: false`.

The previous October report is retained as history, but is invalid as R52
evidence: it ran a Thumb Cortex-M fixture because the descriptor omitted its
core and fell through to the Cortex-M default. See the
[historical fallback report](../coverage/renode-onboarding/attempt-history/cortex-r52_smp/2026-10-18-arm-fallback-report.md).

## Blocker and scope

The native ARM backend includes Cortex-R5F and ARM7TDMI models, not Cortex-R52.
An R5F alias or CPU stub would not establish R52 behavior. A real ARMv8-R CPU
model and compatible ELF execution path are required before this platform can
be measured.

The current inventory record is under
[`docs/coverage/renode-onboarding/metrics/cortex-r52_smp/`](../coverage/renode-onboarding/metrics/cortex-r52_smp/).
This is local repository inventory evidence; it is not a hosted
`labwired_verify` verdict or hardware confirmation.
