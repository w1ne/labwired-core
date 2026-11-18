# Count-preserving Poll-Loop Fast-Forward Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve reported instruction counts when the machine fast-forwards a proven busy loop.

**Architecture:** Keep CPU loop proof and deadline clamping in place. Make CPU fast-forward report the retired instruction count it represents, then carry that count beside skipped cycles through the machine advance result. WFI and timer-poll coalescing remain time-only skips.

**Tech Stack:** Rust, `labwired-core`, Cortex-M poll-loop detector, `Machine::advance`, Work-1 validation.

---

### Task 1: Pin the report contract with a real mbed idle-loop comparison

**Files:**
- Modify: `crates/core/src/cpu/cortex_m/poll_loop.rs` test module

- [x] Add `mbed_idle_loop_fast_forward_preserves_reported_instruction_count` using the existing 8-instruction mbed critical-section loop fixture (`cpsid; ldr; adds; str; subs; str; cpsie; b head`). Build two `Machine<CortexM>` instances from identical CPU and bus state. Enable idle fast-forward on one and disable it on the reference. Advance both to the same 256-cycle/fuel limit.
- [x] Assert equal `primary_steps`, `fuel_consumed`, `elapsed_cycles`, final CPU snapshots, RAM contents, and `step_profile.cpu_instructions`. When `crate::cycle_clock::event_scheduler_enabled()` is true, assert the accelerated report has nonzero `idle_cycles`; when it is false, assert both runs complete through ordinary steps.
- [x] Work-1 event-scheduler test failed first with `primary_steps` 0 vs 256, then passed after accounting was added.

### Task 2: Return poll-loop instruction counts from CPU fast-forward

**Files:**
- Modify: `crates/core/src/cpu/trait_def.rs`
- Modify: `crates/core/src/cpu/cortex_m/poll_loop.rs`
- Modify: `crates/core/src/cpu/cortex_m.rs`
- Modify: `crates/core/src/cpu/xtensa_lx7/poll_loop.rs`
- Modify: `crates/core/src/cpu/xtensa_lx7.rs`

- [x] Change `Cpu::fast_forward_poll_loop` and its forwarding implementation for `&mut C` to return `Option<u64>`: `Some(steps)` means the proven plan was applied and `steps` is the number of retired instructions represented; `None` means no plan was applied.
- [x] In Cortex-M, return `Some(cycles)` only when the stored plan matches the current loop head and the skip is a whole period. Cortex-M's poll-loop model advances one cycle per retired instruction.
- [x] In Xtensa LX7, return `Some(cycles)` only when the stored plan accepts the skip. Its modeled CCOUNT also advances once per retired instruction.
- [x] Updated CPU-level return assertions; the machine-level red test confirmed the omission was report accounting, not CPU plan application.

### Task 3: Account for fast-forwarded steps in machine reports

**Files:**
- Modify: `crates/core/src/lib.rs` (`Machine::try_idle_fast_forward`)
- Modify: `crates/core/src/machine/advance.rs` (skip result consumption)
- Modify: `crates/core/src/machine/mod.rs` (advance report documentation)

- [x] Add an internal skip result with `cycles`, `primary_steps`, and `secondary_steps`; WFI and timer-poll skips retain zero step counts.
- [x] Capture CPU-reported steps for successful primary or secondary poll-loop applications; existing event/deadline and store-observability gates are unchanged.
- [x] Both advance skip call sites keep cycle/fuel telemetry and add represented steps to reports/profile without incrementing `cpu_batches`.
- [x] The differential test passed with `event-scheduler`; feature-off `poll_loop` tests passed with the ordinary execution path.

### Task 4: Verify neighboring paths and performance scope

**Files:**
- No additional source files unless a regression is found

- [x] Work-1 `poll_loop` (11/11), WFI/zero-retirement fast-forward regressions (2/2), and cfg ratchet passed with `event-scheduler`.
- [x] Work-1 feature-off `poll_loop` (11/11) and event-scheduler cfg ratchet (1/1) passed.
- [x] Checked Work-1 Git hook configuration; this checkout has no pre-commit, commit-msg, or pre-push hooks installed.
- [x] Skipped the RP2040 performance gate: this follow-up changes accounting only and makes no speed claim; no baseline was changed.
- [x] The differential test compares instruction steps, fuel, cycles, CPU snapshot, and RAM. CI/merge/deployment/physical acceptance remain separate evidence gates.
