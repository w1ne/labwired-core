# Host time mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development.

**Goal:** Named host policy `max-speed` vs `realtime`; guest time stays Hz.

**Architecture:** `HostTimeMode` + `HostClock` on `Machine`; pace at the end of each committed unit in `advance`. Tests use a fake clock.

**Work from:** `/tmp/labwired-time-mode` branch `feat/host-time-mode`.

---

### Task 1: Enum, clock, pace helper, unit tests

**Files:**
- Modify: `crates/core/src/config.rs`
- Create: `crates/core/src/host_time.rs`
- Modify: `crates/core/src/lib.rs` (mod + re-export)

- [ ] Tests in `host_time.rs` `#[cfg(test)]`:
  - 1_000_000 cycles, 1_000_000 Hz, Realtime, fake now=0 → sleep ≈ 1s − 1ms slack (or full 1s if slack applied as “sleep if ahead by ≥ 1ms”, so sleep 999ms). Spec: threshold 1ms, sleep `virt.saturating_sub(wall)`.
  - MaxSpeed → no sleep
  - cpu_hz 0 → no sleep

- [ ] Implement `HostTimeMode::{MaxSpeed, Realtime}` default MaxSpeed on `SimulationConfig` with `#[serde(default)]`.
- [ ] `HostClock` trait: `fn now(&self) -> Duration` (monotonic from an origin) and `fn sleep(&self, Duration)`.
- [ ] `StdHostClock` using `Instant`.
- [ ] `pace(mode, cpu_hz, start_cycles, now_cycles, start_wall, now_wall, clock)`.
- [ ] Commit `feat(core): host time mode max-speed vs realtime`

---

### Task 2: Wire into `Machine::advance` + CLI

**Files:**
- Modify: `crates/core/src/machine/advance.rs` — after each committed batch/idle, if Realtime, pace using `self.bus.cpu_hz` and a `realtime_origin` captured at the start of `advance`.
- Modify: `crates/core/src/lib.rs` `Machine` — field `host_clock: Box<dyn HostClock>` default Std, or store Instant on Machine only for std and inject via `with_host_clock` for tests.
- Modify: `crates/cli/src/lib.rs` or clap args — `--time-mode max-speed|realtime` on `run`.

Keep `advance` tests passing. A Machine test: Realtime + FakeClock + spin firmware or empty steps that advance cycles.

- [ ] Commit `feat(cli): --time-mode max-speed|realtime`

---

Do not change tick interval or idle-ff.
