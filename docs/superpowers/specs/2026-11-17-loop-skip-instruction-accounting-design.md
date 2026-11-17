# Count-preserving poll-loop fast-forward design

**Status:** Accepted from the user's performance roadmap on 17 Nov 2026.

## Goal

Keep the existing exact poll-loop fast-forward while reporting the skipped retired instructions as well as the elapsed cycles.

## Current behavior

The Cortex-M poll-loop detector already proves a fixed-point iteration on a side-effect-free bus view. It refuses device accesses, visible stores, pending exceptions, changing architectural state, and loops longer than its instruction limit. `Machine::try_idle_fast_forward` clamps a proven skip to the next interrupt, scheduler, motor, and resident-edge deadline and skips a whole number of iterations.

The machine advances `total_cycles`, fuel, and `idle_cycles`, but the skip path does not add the skipped loop instructions to `AdvanceReport.primary_steps` or `StepProfile.cpu_instructions`. WFI and timer-poll coalescing also use this path, but those operations retire no CPU instructions.

## Design

- Preserve the existing loop detector, proof conditions, whole-iteration rule, wake-source checks, and event-deadline clamping.
- Have a successful CPU poll-loop fast-forward return the number of retired instructions it represented. A failed or unavailable plan returns no count and cannot be committed as a skip.
- Carry elapsed cycles, primary skipped steps, and secondary skipped steps together from `Machine::try_idle_fast_forward` to the advance loop.
- Add skipped busy-loop instructions to `AdvanceReport` step counters and `StepProfile.cpu_instructions`. Keep `fuel_consumed` and elapsed-cycle handling as they work today. Do not count a synthetic CPU batch.
- WFI and timer-poll-only skips report zero CPU steps. `idle_cycles` remains skip telemetry and may differ from a reference run.
- Do not synthesize per-instruction observer events. The existing proof still guarantees the skipped loop has no visible intermediate memory or device effects.

## Acceptance

A machine-level differential test runs the real mbed idle-hook loop shape with fast-forward enabled and disabled. The two runs must match on primary instruction count, fuel consumed, elapsed cycles, final CPU snapshot, and RAM. The accelerated run must show skipped idle cycles when event scheduling is enabled. Existing tests continue to guard refusal on observable activity and deadlines.

No deployment or physical-hardware claim is part of this change. The RP2040 performance gate is measured without updating its stored baseline.
