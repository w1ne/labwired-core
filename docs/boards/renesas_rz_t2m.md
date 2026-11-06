# Renesas RZ/T2M validation status

## Current result

The imported RZ/T2M descriptor previously showed `verified: true` based on a
stale run of a Thumb Cortex-M fixture. It did not validate the platform's dual
Cortex-R52 processors.

The bounded onboarding profile now identifies `core: cortex-r52`, retains the
imported 64 MiB flash and 1.5 MiB SRAM windows, and omits the untested SCI and
generic-timer claims. A direct ARM-state integer-loop smoke was attempted
against the exact imported system. LabWired rejected the unsupported CPU core
before executing instructions:

```text
chip 'renesas_rz_t2m': unsupported ARM real-time core 'cortex-r52'; only cortex-r5f has a native adapter
```

The scoreboard records this as an attempted run that did not boot and did not
pass its smoke check. Evidence, including the result JSON, JUnit report,
snapshot, and stage logs, is under
[`docs/coverage/renode-onboarding/metrics/renesas_rz_t2m/`](../coverage/renode-onboarding/metrics/renesas_rz_t2m/).

## Scope

This is local LabWired CLI evidence of an unsupported Cortex-R52 core. It does
not demonstrate RZ/T2M boot, instruction support, SCI/timer behavior, Renode
execution, hosted `labwired_verify` model verification, or hardware behavior.
No CPU or peripheral stub was added. A real Cortex-R52 adapter is needed
before this imported platform can boot.
