# Host time mode (`max-speed` / `realtime`)

**Date:** 2026-09-15  
**Guest clock unchanged:** 1 insn = 1 cycle at `bus.cpu_hz`. Tick interval and idle-ff are not this feature.

## Problem

Simics and Renode name a **host** policy: run as fast as the host can, or sleep so virtual time does not run ahead of wall time. LabWired already runs max-speed; playground/UART demos that should “feel like the board” have no first-class switch.

## Design

`HostTimeMode` on `SimulationConfig`:

- `MaxSpeed` (default) — current behavior; never sleep.
- `Realtime` — after committed work, if `cycles / cpu_hz` is ahead of wall time by ≥ 1 ms, sleep the difference.

Clock is injectable (`HostClock`: `now()` + `sleep()`) so tests never wait on the OS.

Pace from `Machine::advance` after each committed CPU/idle unit, using `bus.cpu_hz`. If `cpu_hz == 0`, Realtime is a no-op (cannot convert).

Origin (`start_cycles`, `start_wall`) is the first Realtime pace in that `advance` call (not Machine construction), so `run_for(1s)` realtime means ~1s wall for that call.

CLI: `--time-mode max-speed|realtime` on `run` (and `test` if a flag already exists for similar opts). Default max-speed.

## Non-goals

- Changing tick 512, idle-ff, or Hz.
- Adaptive icount / MIPS.
- Sleeping every instruction.

## Tests

Fake clock: 1e6 cycles at 1 MHz in Realtime records ~1s sleep (minus 1 ms slack). MaxSpeed records no sleep. `cpu_hz == 0` records no sleep.
