# Unified Machine Advance Design

**Date:** 2026-07-16

## Purpose

LabWired currently advances simulated time through several independent loops:
`Machine::step`, `DebugControl::run`, the CLI test runner's direct
`Cpu::step_batch` path, WASM wrappers, and specialized snapshot/ESP loops. These
paths implement different subsets and orderings of the machine lifecycle. The
result is path-dependent behavior for dual-core execution, scheduler delivery,
software reset, flash operations, logic capture, clocks, and profiling.

This change introduces one authoritative machine-advance operation and migrates
the ordinary core, CLI, and WASM execution paths onto it. The migration is
incremental: the legacy behavior remains available to tests until each new path
has passed architecture-specific fidelity comparisons.

## Goals

1. Add one `Machine::advance` operation that owns simulated execution and every
   machine lifecycle boundary.
2. Preserve the existing `Machine<C: Cpu>` generic and existing public
   `step`, `run`, and WASM APIs through compatibility adapters.
3. Make dual-core execution work through batched/debug execution, preserving
   the existing CPU0/CPU1 round-robin ordering.
4. Remove the CLI test runner's direct `machine.cpu.step_batch` lifecycle
   implementation.
5. Make ordinary WASM stepping use the same operation as native execution.
6. Run targeted tests and explicit old-versus-new fidelity gates after every
   implementation commit.

## Non-goals

This first slice does not:

- split the classic ESP32 LX6 and ESP32-S3 LX7 CPU profiles;
- redesign `Cpu`, `Peripheral`, or `Bus` into capability traits;
- complete multicore runtime snapshots;
- migrate hardware-oracle loops that intentionally execute a bare CPU;
- move the experimental browser Xtensa JIT into the core CPU backend;
- migrate the specialized CLI snapshot-capture loop;
- change documented public CLI or JavaScript argument/return shapes.

The browser JIT and snapshot-capture loops remain explicitly identified legacy
callers. They will be migrated in follow-up designs after the ordinary engine
path is stable.

## Current Behavioral Divergence

The existing paths differ in observable ways:

- `Machine::step` advances CPU0 and CPU1; `DebugControl::run` and the CLI direct
  batch path advance only CPU0.
- `Machine::step` drains classic ESP32 `RTC_CNTL` reset; `run` and CLI batching
  do not.
- CLI batching omits scheduler draining and pending flash operations.
- Lifecycle ordering is inconsistent: single-step uses RTC reset, SCB reset,
  flash, then logic observation; `run` uses flash, SCB reset, then logic; CLI
  batching observes logic before SCB reset and handles neither RTC nor flash.
- CLI batching executes the complete CPU batch before replaying crossed
  peripheral ticks, allowing interrupts and device completion to arrive late.
- CLI batching does not publish the bus clock before CPU execution and does not
  update `StepProfile`.
- Breakpoint semantics differ between the debugger and CLI.

Where legacy paths disagree, repeated legacy `Machine::step` is the lifecycle
correctness oracle because it is the only current path that executes the full
single-instruction lifecycle and both CPUs. A migration test must distinguish
byte-identical refactoring from intentional correction of a known omission.

## Public API

The new API is additive. Existing methods remain available.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointPolicy {
    Ignore,
    Honor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdlePolicy {
    Disabled,
    Configured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchPolicy {
    Auto,
    AtMost(std::num::NonZeroU32),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdvanceLimits {
    pub fuel: Option<u64>,
    pub simulated_cycles: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdvanceMode {
    Single,
    Run,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdvanceRequest {
    limits: AdvanceLimits,
    breakpoints: BreakpointPolicy,
    idle: IdlePolicy,
    batching: BatchPolicy,
    mode: AdvanceMode,
}

impl AdvanceRequest {
    pub fn single() -> Self;
    pub fn run(fuel: Option<u64>) -> Self;
    pub fn with_cycle_limit(self, cycles: u64) -> Self;
    pub fn with_batch_cap(self, cap: std::num::NonZeroU32) -> Self;
    pub fn with_breakpoints(self, policy: BreakpointPolicy) -> Self;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdvanceStop {
    FuelLimit,
    CycleLimit,
    Breakpoint(u32),
    NoProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdvanceReport {
    pub stop: AdvanceStop,
    pub fuel_consumed: u64,
    pub primary_steps: u64,
    pub secondary_steps: u64,
    pub elapsed_cycles: u64,
    pub idle_cycles: u64,
    pub cpu_batches: u64,
}

impl<C: Cpu> Machine<C> {
    pub fn advance(
        &mut self,
        request: AdvanceRequest,
    ) -> SimResult<AdvanceReport>;
}
```

`AdvanceMode` is a private boundary-timing discriminator, not a public policy.
The constructors set it and the builder methods preserve it, so orthogonal
customization such as `AdvanceRequest::single().with_cycle_limit(1)` retains
single-step cycle publication. Production code uses a crate-private predicate
instead of comparing the complete request value.

`fuel` preserves the existing debugger contract: one unit is consumed by a
primary CPU scheduling quantum or an idle-fast-forwarded cycle. It is not named
"instructions" because current `run(max_steps)` counts idle skips against the
same limit.

`simulated_cycles` is checked at committed machine boundaries. CPU planning
never knowingly crosses the remaining budget, but an instruction that lands on
a peripheral tick may reveal an indivisible tick cost only while committing
that boundary. The report therefore stops at the first committed boundary at or
beyond the limit; `elapsed_cycles` may exceed the requested value by that atomic
peripheral cost. This is deterministic boundary overshoot, not an additional
CPU quantum.

`AdvanceRequest::single()` means one primary scheduling quantum, breakpoint
ignore, idle fast-forward disabled, and batch cap one. This preserves debugger
single-step behavior.

The report separates primary and secondary steps while retaining the existing
machine convention that `total_cycles` advances on primary scheduling quanta
and peripheral costs, not once per core.

## Engine Decomposition

The implementation is split into focused machine modules rather than expanding
`crates/core/src/lib.rs`:

- `crates/core/src/machine/mod.rs`: public request/report types and re-exports.
- `crates/core/src/machine/advance.rs`: outer advance loop and stop accounting.
- `crates/core/src/machine/plan.rs`: breakpoint, budget, tick, scheduler, logic,
  cycle-accuracy, and caller-cap batch planning.
- `crates/core/src/machine/boundary.rs`: CPU execution and post-execution
  lifecycle commit.

The existing `Machine` type may remain declared in `lib.rs` during this slice to
avoid a large public-path move. Its execution implementation delegates into the
new modules.

The engine has three internal phases:

```rust
fn plan_batch(&mut self, state: &AdvanceState) -> u32;
fn execute_cpu_window(&mut self, count: u32) -> SimResult<CoreProgress>;
fn commit_boundary(
    &mut self,
    batch_start: u64,
    progress: CoreProgress,
) -> SimResult<()>;
```

### Batch planning

Planning clamps the next CPU window to all applicable boundaries:

- remaining fuel and simulated-cycle budget;
- next peripheral tick;
- `SystemBus::requires_cycle_accurate`;
- active breakpoint policy;
- polled logic capture;
- HC-SR04 deadline;
- general event-scheduler deadline;
- caller batch cap;
- one instruction whenever `cpu_secondary` is present.

Idle fast-forward remains opt-in through `IdlePolicy::Configured`. It retains
the current safety gates, deadline clamp, scheduler advance, bus clock update,
and skipped-cycle telemetry.

### CPU execution

Run-mode single-core machines retain `Cpu::step_batch`, including JIT
eligibility and its per-instruction clock rails. Single-step requests use
`Cpu::step` so their already-published end boundary is not bumped a second time.
Dual-core machines always execute one direct primary instruction, drain and
apply a pending APP_CPU release address, then execute one direct secondary
instruction on the same bus. Their one-quantum run boundary follows repeated
`Machine::step` timing. This preserves the existing single-step ordering and
prevents a CPU0 batch from starving CPU1.

The first slice preserves the existing `Cpu::step_batch` error contract. Exact
partial-retirement reporting on a mid-batch error requires a separate `Cpu`
trait change and is deferred. Characterization tests record the current
behavior so the limitation is explicit.

### Boundary commit

Every successful CPU window commits in this fixed order:

1. cycle, fuel, and `StepProfile` accounting;
2. peripheral ticks, tick costs, observer callbacks, and returned IRQs;
3. scheduler time publication and due-event drain;
4. classic ESP32 RTC software-reset drain;
5. Cortex-M SCB software-reset drain;
6. pending flash operation application;
7. logic push drain and poll observation;
8. structured report update.

This is the current complete single-step ordering. Moving flash before reset or
logic before reset would be an intentional semantic change and is not part of
this refactor.

## Compatibility Adapters

`Machine::step` delegates to `advance(AdvanceRequest::single())` and discards the
report.

`DebugControl::run` delegates to `advance(AdvanceRequest::run(...))` and maps
`AdvanceStop` back into the existing `StopReason`. No variants are added to
`StopReason`, avoiding downstream exhaustive-match breakage.

`DebugControl::step_single` delegates to `AdvanceRequest::single()`.

An additive object-safe `ExecutionControl` trait may expose `advance` to dynamic
consumers without changing required methods on `DebugControl`:

```rust
pub trait ExecutionControl: DebugControl {
    fn advance(&mut self, request: AdvanceRequest) -> SimResult<AdvanceReport>;
}
```

DAP, GDB, and Python can remain on `DebugControl` until they need structured
advance reports.

## Frontend Migration

### CLI

The CLI retains ownership of host-side policy:

- wall-time and UART-byte limits;
- assertions and durable assertion settling;
- input stimuli and fault injection;
- app-entry snapshot file creation;
- artifact serialization.

For each outer-loop iteration it calculates the nearest host boundary and calls
`Machine::advance`. The direct `machine.cpu.step_batch` branch and its manual
tick/reset/logic code are removed after dual-path fidelity passes.

### WASM

Ordinary `step`, `step_single`, `step_batch`, and batch-profile methods use
`Machine::advance`. Existing JavaScript-visible return values are preserved;
additional report fields may be added to profile JSON without removing or
renaming current fields.

`step_with_esp32_aids` remains on the single-step adapter during this slice so
its DPORT bridge stays at instruction boundaries. Its call-relative handshake
cadence and experimental browser JIT are documented follow-up defects. The
browser JIT remains disabled by default and is not used as a fidelity oracle.

### Specialized loops

CLI snapshot capture and hardware-oracle loops are not silently rewritten.
They remain searchable direct-CPU callers and receive follow-up migration specs.

## Fidelity Strategy

Every implementation task follows red-green TDD and runs fidelity verification
before its commit. Legacy and unified paths must both execute non-vacuously.
Comparisons use two independently constructed machines; one live machine is not
rewound because current runtime snapshots omit CPU1 and scheduler state.

The comparison state includes, where supported:

- primary and secondary CPU snapshots;
- sorted peripheral snapshots;
- RAM and extra-memory contents or hashes;
- `total_cycles`, scheduler time/deadlines, pending IRQ state, and profiles;
- exact UART bytes, bus-trace events, logic edges and ordering;
- stop reason and advance report;
- CLI `result.json`, snapshots, JUnit, trace/VCD artifacts, excluding only
  wall-clock durations and fields explicitly documented as non-canonical.

### Gate 1: characterization only

Add tests before production changes for:

- repeated `step` versus `run` on a simple Cortex-M machine;
- exact bus-cycle publication and peripheral-cost accounting;
- breakpoint stickiness and no-progress behavior;
- dual-core CPU0/CPU1 round-robin and APP_CPU release;
- RTC, SCB, and flash boundary handling;
- current legacy divergences that later tasks intentionally correct.

Run:

```bash
cargo test -p labwired-core machine_advance_characterization -- --nocapture
cargo test -p labwired-core scb_reset -- --nocapture
```

### Gate 2: unified single-step

Compare legacy single-step and `advance(single)` after every boundary for ARM,
RISC-V, and Xtensa fixture machines, including reset, flash, logic capture, and
dual-core state.

Run:

```bash
cargo test -p labwired-core machine_advance_single -- --nocapture
cargo test -p labwired-core scb_reset logic_capture -- --nocapture
cargo test -p labwired-core --test runtime_snapshot -- --nocapture
```

### Gate 3: unified batched run

Compare legacy `run` and unified batching where legacy behavior is valid. For
dual-core and RTC reset, compare unified batching against repeated single-step
and assert the intentional correction.

Run:

```bash
cargo test -p labwired-core machine_advance_run -- --nocapture
cargo test -p labwired-core --features event-scheduler \
  --test systick_walk_differential \
  --test stm32_timer_walk_differential \
  --test stm32_dma_walk_differential \
  --test esp32s3_walk_differential -- --nocapture
cargo test --release -p labwired-core --features jit \
  --test riscv_jit_c3_oled_differential -- --nocapture
```

The ignored, fixture-backed C3 full-state gates run when their documented
firmware assets are available:

```bash
cargo test --release -p labwired-core --features event-scheduler \
  --test esp32c3_walk_differential -- --ignored --nocapture
cargo test --release -p labwired-core --features event-scheduler \
  --test esp32c3_clamped_full_state_differential \
  oled_lab_full_state_byte_identical_interval_1_vs_64 -- --ignored --nocapture
```

### Gate 4: CLI migration

The CLI dual-path harness compares result artifacts, UART, stop reason,
assertion/stimulus timing, logic edges, and JIT statistics.

Run:

```bash
cargo test -p labwired-cli \
  --test runner --test outputs --test snapshots \
  --test interactive_snapshot --test determinism --test golden_examples \
  -- --nocapture
cargo test --release -p labwired-cli --features jit-core \
  --test riscv_jit_c3_oled_test_differential \
  --test riscv_tick_interval_fidelity_differential -- --nocapture
```

### Gate 5: WASM migration

Host tests compare ordinary step and batch behavior for representative ARM,
RISC-V, and Xtensa machines. The wasm32 target must compile.

Run:

```bash
cargo test -p labwired-wasm
cargo check -p labwired-wasm --target wasm32-unknown-unknown
```

Browser runtime equivalence remains a documented gap until the experimental
browser JIT is moved behind the CPU execution contract.

### Final gate

After all legacy ordinary paths are removed:

```bash
cargo test -p labwired-core
cargo test -p labwired-core --features event-scheduler
cargo test -p labwired-core --features jit,event-scheduler
cargo test -p labwired-cli
cargo test -p labwired-cli --features jit-core
cargo test -p labwired-wasm
cargo check -p labwired-wasm --target wasm32-unknown-unknown
cargo fmt --all -- --check
cargo clippy -p labwired-core -p labwired-cli -p labwired-wasm --all-targets -- -D warnings
```

Fixture-backed ignored tests are reported separately if assets are absent; they
are not silently described as passing.

## Error Handling

- `Machine::advance` returns `SimResult<AdvanceReport>` and preserves current
  CPU errors.
- A breakpoint or exhausted budget is a normal `AdvanceStop`, not an error.
- `Ok(0)` CPU progress maps to `AdvanceStop::NoProgress` so callers cannot spin
  indefinitely.
- Reset and flash failures propagate as `SimulationError` after all state
  changes that precede them in the defined boundary order.
- Frontends map structured stops into their existing exit/status schemas.

## Rollout and Commit Boundaries

1. Add characterization and fidelity harnesses only.
2. Add request/report types and unified single-step internals; retain legacy
   single-step for comparison.
3. Make `Machine::step` an adapter after single-step fidelity passes.
4. Add safe batch planning and migrate `DebugControl::run`; explicitly fix and
   test dual-core and RTC reset omissions.
5. Migrate the CLI test runner and remove its manual lifecycle batch.
6. Migrate ordinary WASM stepping and profiling.
7. Remove test-only legacy ordinary paths after the complete matrix passes.
8. Run formatting, lint, default, scheduler, JIT, CLI, and WASM final gates.

Each boundary is independently reviewable and revertible. No task proceeds to
the next boundary until its targeted tests and fidelity comparison pass.

## Success Criteria

The slice is complete when:

1. `Machine::step`, `DebugControl::run`, the CLI test runner, and ordinary WASM
   stepping all delegate to `Machine::advance`.
2. No ordinary CLI or WASM path calls `machine.cpu.step` or
   `machine.cpu.step_batch` directly.
3. Dual-core execution advances CPU1 through `run` and batch APIs.
4. RTC, SCB, scheduler, flash, clock, profiling, and logic boundary behavior is
   owned by one implementation.
5. Every migration commit has recorded targeted test and fidelity evidence.
6. The final default, scheduler, JIT, CLI, WASM, formatting, and lint gates pass,
   with fixture-dependent omissions reported explicitly.
7. Direct CPU stepping remains only in documented specialized or oracle code.
