# Unified Machine Advance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace LabWired's divergent ordinary execution loops with one fidelity-tested `Machine::advance` lifecycle used by core, CLI, and WASM.

**Architecture:** Add request/report types plus focused machine modules for planning, CPU-window execution, and boundary commit. Preserve `Machine<C: Cpu>` and current public wrappers; migrate one caller at a time while test-only legacy paths and architecture-specific differentials prove fidelity. Repeated legacy `Machine::step` is the oracle where existing batch paths omit lifecycle behavior.

**Tech Stack:** Rust 2021, Cargo workspace, serde, wasm-bindgen, LabWired snapshot/trace/logic/JIT fidelity harnesses.

---

## File Structure

- Create `crates/core/src/machine/mod.rs`: advance request/report types and module wiring.
- Create `crates/core/src/machine/advance.rs`: authoritative outer loop and stop/report accounting.
- Create `crates/core/src/machine/plan.rs`: safe batch-width and idle-fast-forward planning.
- Create `crates/core/src/machine/boundary.rs`: CPU0/CPU1 execution and lifecycle commit.
- Create `crates/core/src/tests/machine_advance.rs`: characterization and differential tests.
- Modify `crates/core/src/lib.rs`: re-exports plus `step`/`run` delegation.
- Modify `crates/cli/src/main.rs`: replace both bespoke test-run execution branches.
- Create `crates/cli/tests/machine_advance_fidelity.rs`: artifact/UART parity.
- Modify `crates/wasm/src/lib.rs`: migrate ordinary wrappers without JS API breakage.
- Modify `docs/architecture.md`: document the authoritative lifecycle.

## Task 1: Freeze Current Execution Semantics

**Files:**
- Create: `crates/core/src/tests/machine_advance.rs`
- Modify: `crates/core/src/tests/mod.rs`
- Modify: `crates/core/src/tests/test_cycles.rs`

- [ ] **Step 1: Register the test module**

```rust
#[cfg(test)]
pub mod machine_advance;
```

- [ ] **Step 2: Add a deterministic CPU double**

Implement this shape in `machine_advance.rs`, completing every required `Cpu` method with deterministic field access copied from `test_cycles.rs`—no `todo!`, `unimplemented!`, or panics:

```rust
#[derive(Default)]
struct CountingCpu {
    pc: u32,
    sp: u32,
    steps: u32,
    pending: Vec<u64>,
    halted: bool,
}

impl Cpu for CountingCpu {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        self.pc = 0;
        self.sp = 0;
        self.steps = 0;
        self.pending.clear();
        self.halted = false;
        Ok(())
    }
    fn step(&mut self, _bus: &mut dyn Bus, _observers: &[Arc<dyn SimulationObserver>], _config: &SimulationConfig) -> SimResult<()> {
        if !self.halted {
            self.steps += 1;
            self.pc = self.pc.wrapping_add(2);
        }
        Ok(())
    }
    fn set_pc(&mut self, value: u32) { self.pc = value; }
    fn get_pc(&self) -> u32 { self.pc }
    fn set_sp(&mut self, value: u32) { self.sp = value; }
    fn halt(&mut self) { self.halted = true; }
    fn unhalt(&mut self) { self.halted = false; }
    fn set_exception_pending(&mut self, n: u32) { self.pending.push(u64::from(n)); }
    fn get_register(&self, id: u8) -> u32 { match id { 0 => self.steps, 13 => self.sp, 15 => self.pc, _ => 0 } }
    fn set_register(&mut self, id: u8, value: u32) { match id { 0 => self.steps = value, 13 => self.sp = value, 15 => self.pc = value, _ => {} } }
    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        let mut registers = vec![0; 16];
        registers[0] = self.steps;
        registers[13] = self.sp;
        registers[15] = self.pc;
        crate::snapshot::CpuSnapshot::Arm(crate::snapshot::ArmCpuSnapshot {
            registers, pc: self.pc, xpsr: 0, primask: false,
            pending_exceptions: 0, pending_exceptions_hi: self.pending.clone(), vtor: 0,
        })
    }
    fn apply_snapshot(&mut self, snapshot: &crate::snapshot::CpuSnapshot) {
        if let crate::snapshot::CpuSnapshot::Arm(arm) = snapshot {
            self.pc = arm.pc;
            self.steps = arm.registers[0];
            self.sp = arm.registers[13];
            self.pending.clone_from(&arm.pending_exceptions_hi);
        }
    }
    fn get_register_names(&self) -> Vec<String> { (0..16).map(|n| format!("r{n}")).collect() }
    fn index_of_register(&self, name: &str) -> Option<u8> { name.strip_prefix('r')?.parse().ok() }
}

fn counting_dual_core_machine() -> Machine<CountingCpu> {
    Machine::new(CountingCpu::default(), SystemBus::new())
        .with_secondary_cpu(CountingCpu::default())
}
```

- [ ] **Step 3: Characterize current dual-core and accounting behavior**

```rust
#[test]
fn legacy_step_advances_both_cores_once() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new())
        .with_secondary_cpu(CountingCpu::default());
    machine.step().unwrap();
    assert_eq!(machine.cpu.steps, 1);
    assert_eq!(machine.cpu_secondary.as_ref().unwrap().steps, 1);
    assert_eq!(machine.total_cycles, 1);
}

#[test]
fn legacy_run_currently_omits_secondary_core() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new())
        .with_secondary_cpu(CountingCpu::default());
    assert_eq!(machine.run(Some(4)).unwrap(), StopReason::MaxStepsReached);
    assert_eq!(machine.cpu.steps, 4);
    assert_eq!(machine.cpu_secondary.as_ref().unwrap().steps, 0);
}

#[test]
fn legacy_single_step_publishes_and_profiles_one_cycle() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new());
    machine.reset_step_profile();
    machine.step().unwrap();
    assert_eq!(machine.total_cycles, 1);
    assert_eq!(machine.bus.current_cycle(), 1);
    assert_eq!(machine.step_profile().cpu_instructions, 1);
    assert_eq!(machine.step_profile().cpu_batches, 1);
}

#[test]
fn legacy_dual_core_halted_primary_still_consumes_one_scheduling_quantum() {
    let mut machine = counting_dual_core_machine();
    machine.cpu.halt();
    machine.step().unwrap();
    assert_eq!(machine.cpu.steps, 0);
    assert_eq!(machine.cpu_secondary.as_ref().unwrap().steps, 1);
    assert_eq!(machine.total_cycles, 1);
    assert_eq!(machine.step_profile().cpu_instructions, 1);
}
```

Extend `test_cycles.rs` with breakpoint stickiness: set PC to `0x1001`, add breakpoint `0x1000`, assert the first `run` stops, call `run(Some(1))` and assert PC changed, explicitly restore PC to `0x1001`, then assert a third `run` stops at the same breakpoint again.

- [ ] **Step 4: Run characterization**

```bash
cargo test -p labwired-core machine_advance -- --nocapture
cargo test -p labwired-core test_machine_run_cycles -- --nocapture
cargo test -p labwired-core machine_run_records_step_profile_counters -- --nocapture
cargo test -p labwired-core scb_reset -- --nocapture
```

Expected: PASS, including the test documenting legacy CPU1 omission.

- [ ] **Step 5: Commit tests only**

```bash
git add crates/core/src/tests/mod.rs crates/core/src/tests/machine_advance.rs crates/core/src/tests/test_cycles.rs
git commit -m "test(core): characterize machine execution paths"
```

## Task 2: Add the Advance Contract

**Files:**
- Create: `crates/core/src/machine/mod.rs`
- Create: `crates/core/src/machine/advance.rs`
- Create: `crates/core/src/machine/plan.rs`
- Create: `crates/core/src/machine/boundary.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/tests/machine_advance.rs`

- [ ] **Step 1: Write failing constructor tests**

```rust
#[test]
fn single_request_is_one_non_batched_non_idle_quantum() {
    let request = AdvanceRequest::single();
    assert_eq!(request.limits().fuel, Some(1));
    assert_eq!(request.limits().simulated_cycles, None);
    assert_eq!(request.breakpoint_policy(), BreakpointPolicy::Ignore);
    assert_eq!(request.idle_policy(), IdlePolicy::Disabled);
    assert_eq!(request.batch_policy(), BatchPolicy::AtMost(NonZeroU32::new(1).unwrap()));
    assert!(request.is_single());
}

#[test]
fn run_request_preserves_optional_fuel() {
    let request = AdvanceRequest::run(Some(64));
    assert_eq!(request.limits().fuel, Some(64));
    assert_eq!(request.limits().simulated_cycles, None);
    assert_eq!(AdvanceRequest::run(None).limits().fuel, None);
    assert_eq!(request.breakpoint_policy(), BreakpointPolicy::Honor);
    assert_eq!(request.idle_policy(), IdlePolicy::Configured);
    assert_eq!(request.batch_policy(), BatchPolicy::Auto);
    assert!(!request.is_single());
}

#[test]
fn request_builders_override_only_their_policy() {
    let cap = NonZeroU32::new(7).unwrap();
    let request = AdvanceRequest::single()
        .with_cycle_limit(9)
        .with_batch_cap(cap)
        .with_breakpoints(BreakpointPolicy::Honor);
    assert_eq!(request.limits().fuel, Some(1));
    assert_eq!(request.limits().simulated_cycles, Some(9));
    assert_eq!(request.batch_policy(), BatchPolicy::AtMost(cap));
    assert_eq!(request.breakpoint_policy(), BreakpointPolicy::Honor);
    assert_eq!(request.idle_policy(), IdlePolicy::Disabled);
    assert!(request.is_single(), "builders must preserve boundary timing mode");
}
```

- [ ] **Step 2: Verify RED**

```bash
cargo test -p labwired-core single_request_is_one_non_batched_non_idle_quantum -- --nocapture
```

Expected: compile failure because advance types do not exist.

- [ ] **Step 3: Define the approved API**

In `machine/mod.rs`, define `BreakpointPolicy`, `IdlePolicy`, `BatchPolicy`, `AdvanceLimits`, `AdvanceRequest`, `AdvanceStop`, and `AdvanceReport` exactly as the design. Implement:

```rust
impl AdvanceRequest {
    pub fn single() -> Self {
        Self {
            limits: AdvanceLimits { fuel: Some(1), simulated_cycles: None },
            breakpoints: BreakpointPolicy::Ignore,
            idle: IdlePolicy::Disabled,
            batching: BatchPolicy::AtMost(NonZeroU32::new(1).unwrap()),
            mode: AdvanceMode::Single,
        }
    }
    pub fn run(fuel: Option<u64>) -> Self {
        Self {
            limits: AdvanceLimits { fuel, simulated_cycles: None },
            breakpoints: BreakpointPolicy::Honor,
            idle: IdlePolicy::Configured,
            batching: BatchPolicy::Auto,
            mode: AdvanceMode::Run,
        }
    }
    pub fn with_cycle_limit(mut self, cycles: u64) -> Self { self.limits.simulated_cycles = Some(cycles); self }
    pub fn with_batch_cap(mut self, cap: NonZeroU32) -> Self { self.batching = BatchPolicy::AtMost(cap); self }
    pub fn with_breakpoints(mut self, policy: BreakpointPolicy) -> Self { self.breakpoints = policy; self }
    pub fn limits(self) -> AdvanceLimits { self.limits }
    pub fn breakpoint_policy(self) -> BreakpointPolicy { self.breakpoints }
    pub fn idle_policy(self) -> IdlePolicy { self.idle }
    pub fn batch_policy(self) -> BatchPolicy { self.batching }
    pub(crate) fn is_single(self) -> bool { self.mode == AdvanceMode::Single }
}
```

Add responsibility comments to the three child files; behavior lands in later tasks.

- [ ] **Step 4: Export types from core**

```rust
pub mod machine;
pub use machine::{AdvanceLimits, AdvanceReport, AdvanceRequest, AdvanceStop, BatchPolicy, BreakpointPolicy, IdlePolicy};
```

- [ ] **Step 5: Verify GREEN**

```bash
cargo test -p labwired-core single_request_is_one_non_batched_non_idle_quantum -- --nocapture
cargo test -p labwired-core run_request_preserves_optional_fuel -- --nocapture
cargo test -p labwired-core test_cycles -- --nocapture
cargo check -p labwired-core
```

Expected: PASS.

- [ ] **Step 6: Commit additive API**

```bash
git add crates/core/src/lib.rs crates/core/src/machine crates/core/src/tests/machine_advance.rs
git commit -m "feat(core): define machine advance contract"
```

## Task 3: Route Single Steps Through Advance

**Files:**
- Modify: `crates/core/src/machine/advance.rs`
- Modify: `crates/core/src/machine/boundary.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/tests/machine_advance.rs`

- [ ] **Step 1: Preserve a temporary legacy oracle and write the differential**

Move the current `Machine::step` body unchanged to `#[cfg(test)] pub(crate) fn step_legacy_for_test`. Add:

```rust
#[test]
fn advance_single_is_byte_identical_to_legacy_step() {
    let mut legacy = counting_dual_core_machine();
    let mut unified = counting_dual_core_machine();
    legacy.step_legacy_for_test().unwrap();
    let report = unified.advance(AdvanceRequest::single()).unwrap();
    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!((report.primary_steps, report.secondary_steps), (1, 1));
    assert_eq!(serde_json::to_value(unified.cpu.snapshot()).unwrap(), serde_json::to_value(legacy.cpu.snapshot()).unwrap());
    assert_eq!(serde_json::to_value(unified.cpu_secondary.as_ref().unwrap().snapshot()).unwrap(), serde_json::to_value(legacy.cpu_secondary.as_ref().unwrap().snapshot()).unwrap());
    assert_eq!(serde_json::to_value(unified.snapshot()).unwrap(), serde_json::to_value(legacy.snapshot()).unwrap());
    assert_eq!(unified.total_cycles, legacy.total_cycles);
    assert_eq!(unified.step_profile(), legacy.step_profile());
}

fn assert_real_cpu_single_step_matches<C: Cpu>(factory: impl Fn() -> Machine<C>) {
    let mut legacy = factory();
    let mut unified = factory();
    legacy.step_legacy_for_test().unwrap();
    let report = unified.advance(AdvanceRequest::single()).unwrap();
    assert_eq!(report.primary_steps, 1);
    assert_eq!(serde_json::to_value(legacy.cpu.snapshot()).unwrap(), serde_json::to_value(unified.cpu.snapshot()).unwrap());
    assert_eq!(serde_json::to_value(legacy.snapshot()).unwrap(), serde_json::to_value(unified.snapshot()).unwrap());
    assert_eq!(legacy.total_cycles, unified.total_cycles);
    assert_eq!(legacy.bus.current_cycle(), unified.bus.current_cycle());
    assert_eq!(legacy.step_profile(), unified.step_profile());
}

#[test]
fn arm_single_step_matches_legacy_boundary() {
    assert_real_cpu_single_step_matches(|| {
        let mut bus = SystemBus::new();
        let (mut cpu, _) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        bus.write_u16(0, 0xBF00).unwrap();
        cpu.set_pc(0);
        Machine::new(cpu, bus)
    });
}

#[test]
fn riscv_single_step_matches_legacy_boundary() {
    assert_real_cpu_single_step_matches(|| {
        let mut bus = SystemBus::new();
        let mut cpu = crate::system::riscv::configure_riscv(&mut bus);
        bus.write_u32(0, 0x0000_0013).unwrap();
        cpu.set_pc(0);
        Machine::new(cpu, bus)
    });
}

#[test]
fn xtensa_single_step_matches_legacy_boundary() {
    assert_real_cpu_single_step_matches(|| {
        let mut bus = SystemBus::new();
        let mut cpu = crate::cpu::xtensa_lx7::XtensaLx7::new();
        bus.write_u8(0, 0x3d).unwrap();
        bus.write_u8(1, 0xf0).unwrap();
        cpu.set_pc(0);
        Machine::new(cpu, bus)
    });
}

#[test]
fn unified_single_releases_and_steps_app_cpu() {
    let mut machine = counting_dual_core_machine();
    machine.cpu_secondary.as_mut().unwrap().halt();
    crate::peripherals::esp_xtensa_common::rom_thunks::APPCPU_BOOT_ADDR
        .with(|slot| slot.set(Some(0x4008_0000)));
    machine.advance(AdvanceRequest::single()).unwrap();
    let cpu1 = machine.cpu_secondary.as_ref().unwrap();
    assert!(!cpu1.halted);
    assert_eq!(cpu1.pc, 0x4008_0002);
    assert_eq!(cpu1.steps, 1);
}
```

- [ ] **Step 2: Verify RED**

```bash
cargo test -p labwired-core advance_single_is_byte_identical_to_legacy_step -- --nocapture
```

Expected: compile failure because `Machine::advance` is absent.

- [ ] **Step 3: Implement CPU-window execution**

```rust
#[derive(Clone, Copy)]
pub(crate) struct CoreProgress { pub primary_steps: u32, pub secondary_steps: u32 }

impl<C: Cpu> Machine<C> {
    pub(crate) fn execute_cpu_window(&mut self, count: u32) -> SimResult<CoreProgress> {
        if self.cpu_secondary.is_none() {
            let n = self.cpu.step_batch(&mut self.bus, &self.observers, &self.config, count)?;
            return Ok(CoreProgress { primary_steps: n, secondary_steps: 0 });
        }
        debug_assert_eq!(count, 1);
        self.cpu.step(&mut self.bus, &self.observers, &self.config)?;
        self.release_secondary_cpu_if_requested();
        self.cpu_secondary.as_mut().unwrap().step(&mut self.bus, &self.observers, &self.config)?;
        Ok(CoreProgress { primary_steps: 1, secondary_steps: 1 })
    }
}
```

Extract APP_CPU release unchanged into `release_secondary_cpu_if_requested`.

- [ ] **Step 4: Implement one-quantum advance and delegate step**

For `AdvanceRequest::single`, publish the same pre-instruction cycle as legacy step, execute count one, run the unchanged peripheral/scheduler/RTC/SCB/flash/logic sequence, and return the report. Then replace public step with:

```rust
pub fn step(&mut self) -> SimResult<()> {
    self.advance(AdvanceRequest::single()).map(|_| ())
}
```

- [ ] **Step 5: Run per-step fidelity**

```bash
cargo test -p labwired-core advance_single -- --nocapture
cargo test -p labwired-core arm_single_step_matches_legacy_boundary -- --nocapture
cargo test -p labwired-core riscv_single_step_matches_legacy_boundary -- --nocapture
cargo test -p labwired-core xtensa_single_step_matches_legacy_boundary -- --nocapture
cargo test -p labwired-core unified_single_releases_and_steps_app_cpu -- --nocapture
cargo test -p labwired-core scb_reset -- --nocapture
cargo test -p labwired-core logic_capture -- --nocapture
cargo test -p labwired-core --test runtime_snapshot -- --nocapture
cargo test -p labwired-core --test stm32_spi_waveform --test esp32c3_i2c_waveform -- --nocapture
```

Expected: PASS with both legacy and unified arms non-vacuous.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/lib.rs crates/core/src/machine crates/core/src/tests/machine_advance.rs
git commit -m "refactor(core): route single steps through machine advance"
```

## Task 4: Generalize Advance and Replace Debug Run

**Files:**
- Modify: `crates/core/src/machine/advance.rs`
- Modify: `crates/core/src/machine/plan.rs`
- Modify: `crates/core/src/machine/boundary.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/tests/machine_advance.rs`
- Modify: `crates/core/src/tests/scb_reset.rs`

- [ ] **Step 1: Write failing correction tests**

```rust
#[test]
fn unified_run_advances_both_cores_one_quantum_at_a_time() {
    let mut machine = counting_dual_core_machine();
    let report = machine.advance(AdvanceRequest::run(Some(4))).unwrap();
    assert_eq!((report.primary_steps, report.secondary_steps), (4, 4));
    assert_eq!(machine.cpu_secondary.as_ref().unwrap().steps, 4);
}

#[test]
fn run_adapter_advances_secondary_core() {
    let mut machine = counting_dual_core_machine();
    assert_eq!(machine.run(Some(4)).unwrap(), StopReason::MaxStepsReached);
    assert_eq!(machine.cpu_secondary.as_ref().unwrap().steps, 4);
}
```

Add this focused classic RTC test:

```rust
#[test]
fn rtc_reset_request_is_drained_by_run_adapter() {
    use crate::peripherals::esp32::rtc_cntl::{
        RtcCntl, RTC_CNTL_OPTIONS0_OFFSET, RTC_CNTL_OPTIONS0_SW_SYS_RST_BIT,
    };
    let mut bus = SystemBus::new();
    bus.add_peripheral("rtc_cntl", RtcCntl::BASE.into(), 0x200, None, Box::new(RtcCntl::new()));
    let mut machine = Machine::new(CountingCpu::default(), bus);
    machine.bus.write_u32(
        u64::from(RtcCntl::BASE) + RTC_CNTL_OPTIONS0_OFFSET,
        RTC_CNTL_OPTIONS0_SW_SYS_RST_BIT,
    ).unwrap();
    machine.run(Some(1)).unwrap();
    assert_eq!(machine.cpu.get_pc(), 0x4000_0400);
    assert_eq!(machine.cpu.get_register(13), 0x3FFE_0000);
}

#[test]
fn rtc_reset_is_committed_before_later_instructions_in_a_wide_request() {
    use crate::peripherals::esp32::rtc_cntl::{
        RtcCntl, RTC_CNTL_OPTIONS0_OFFSET, RTC_CNTL_OPTIONS0_SW_SYS_RST_BIT,
    };
    let mut bus = SystemBus::new();
    bus.add_peripheral("rtc_cntl", RtcCntl::BASE.into(), 0x200, None, Box::new(RtcCntl::new()));
    let mut machine = Machine::new(CountingCpu::default(), bus);
    machine.config.peripheral_tick_interval = 64;
    machine.bus.write_u32(
        u64::from(RtcCntl::BASE) + RTC_CNTL_OPTIONS0_OFFSET,
        RTC_CNTL_OPTIONS0_SW_SYS_RST_BIT,
    ).unwrap();
    machine.advance(AdvanceRequest::run(Some(8))).unwrap();
    assert_eq!(machine.cpu.get_pc(), 0x4000_0400 + 14);
}
```

In `scb_reset.rs`, replace the reset-vector self-loop in `sysresetreq_reboots_cpu_via_run` with eight Thumb NOPs, set `peripheral_tick_interval = 64`, run eight instructions, and assert the final PC is `RESET_ADDR + 14`. This proves the reset is committed after the first instruction, before the remaining seven, rather than after one wide CPU batch.

- [ ] **Step 2: Verify RED**

```bash
cargo test -p labwired-core unified_run_advances_both_cores_one_quantum_at_a_time -- --nocapture
cargo test -p labwired-core rtc_reset -- --nocapture
```

Expected: old run fails CPU1/RTC assertions.

- [ ] **Step 3: Implement safe planning**

Before implementing the loop, preserve three distinct execution modes. A
single request publishes and seeds the end boundary then calls `Cpu::step`; a
single-core run publishes and seeds the batch start then calls
`Cpu::step_batch`; a dual-core run executes one direct lockstep quantum using
repeated-step end-boundary timing. Do not infer the execution primitive from
`count == 1`, because a capped run batch and a single request have different
clock semantics.

Cycle limits stop at the first committed boundary at or beyond the limit. The
planner does not cross the remaining CPU-cycle budget, but an indivisible
peripheral tick cost discovered during that boundary may deterministically
overshoot it; tests must assert the reported overshoot and that no additional
CPU quantum executes.

```rust
pub(crate) struct AdvanceState { pub fuel_consumed: u64, pub start_cycles: u64 }

impl<C: Cpu> Machine<C> {
    pub(crate) fn plan_advance_batch(&mut self, request: AdvanceRequest, state: &AdvanceState) -> u32 {
        let remaining = request.limits().fuel.map(|n| n.saturating_sub(state.fuel_consumed)).unwrap_or(u64::from(u32::MAX));
        let cycle_remaining = request.limits().simulated_cycles
            .map(|limit| limit.saturating_sub(self.total_cycles - state.start_cycles))
            .unwrap_or(u64::from(u32::MAX));
        let tick = u64::from(self.config.peripheral_tick_interval.max(1));
        let mut count = remaining.min(cycle_remaining).min(tick - self.total_cycles % tick)
            .min(u64::from(u32::MAX)) as u32;
        if let BatchPolicy::AtMost(cap) = request.batch_policy() { count = count.min(cap.get()); }
        if self.cpu_secondary.is_some() || self.rtc_cntl_index.is_some() || self.scb_index.is_some()
            || self.bus.requires_cycle_accurate() || self.logic_poll_active()
            || (request.breakpoint_policy() == BreakpointPolicy::Honor && !self.breakpoints.is_empty()) {
            count = count.min(1);
        }
        #[cfg(feature = "event-scheduler")]
        {
            if count > 1 {
                if let Some(deadline) = self.bus.next_hcsr04_deadline_cycle() {
                    let until = deadline.saturating_sub(self.total_cycles);
                    count = count.min(until.clamp(1, u64::from(u32::MAX)) as u32);
                }
            }
            if self.config.peripheral_tick_interval > 1 && count > 1 {
                self.refresh_generation_scratch();
                if let Some(deadline) = self.sched.next_event_deadline(&self.generation_scratch) {
                    count = if deadline > self.total_cycles {
                        count.min((deadline - self.total_cycles).min(u64::from(u32::MAX)) as u32)
                    } else {
                        1
                    };
                }
            }
        }
        count
    }
}
```

This is an extraction of the existing feature-gated HC-SR04 and general scheduler clamps; keep the math byte-for-byte equivalent to the old run loop while moving it.

- [ ] **Step 4: Generalize the advance loop**

Implement the outer loop in this exact order:

```rust
pub fn advance(&mut self, request: AdvanceRequest) -> SimResult<AdvanceReport> {
    let start_cycles = self.total_cycles;
    let mut fuel_consumed = 0u64;
    let mut primary_steps = 0u64;
    let mut secondary_steps = 0u64;
    let mut idle_cycles = 0u64;
    let mut cpu_batches = 0u64;
    loop {
        let pc = self.cpu.get_pc();
        let aligned = pc & !1;
        if request.breakpoint_policy() == BreakpointPolicy::Honor
            && self.breakpoints.contains(&aligned)
            && self.last_breakpoint != Some(aligned)
        {
            self.last_breakpoint = Some(aligned);
            return Ok(AdvanceReport::new(AdvanceStop::Breakpoint(pc), fuel_consumed,
                primary_steps, secondary_steps, self.total_cycles - start_cycles,
                idle_cycles, cpu_batches));
        }
        self.last_breakpoint = None;
        if request.limits().fuel.is_some_and(|limit| fuel_consumed >= limit) {
            return Ok(AdvanceReport::new(AdvanceStop::FuelLimit, fuel_consumed,
                primary_steps, secondary_steps, self.total_cycles - start_cycles,
                idle_cycles, cpu_batches));
        }
        if request.limits().simulated_cycles
            .is_some_and(|limit| self.total_cycles - start_cycles >= limit)
        {
            return Ok(AdvanceReport::new(AdvanceStop::CycleLimit, fuel_consumed,
                primary_steps, secondary_steps, self.total_cycles - start_cycles,
                idle_cycles, cpu_batches));
        }
        if request.idle_policy() == IdlePolicy::Configured {
            let max_skip = match (request.limits().fuel, request.limits().simulated_cycles) {
                (None, None) => None,
                (fuel, cycles) => Some(
                    fuel.map(|limit| limit.saturating_sub(fuel_consumed)).unwrap_or(u64::MAX)
                        .min(cycles.map(|limit| limit.saturating_sub(self.total_cycles - start_cycles)).unwrap_or(u64::MAX))
                        .min(u64::from(u32::MAX)) as u32,
                ),
            };
            let skipped = self.try_idle_fast_forward(max_skip, 0);
            if skipped > 0 {
                fuel_consumed += u64::from(skipped);
                idle_cycles += u64::from(skipped);
                continue;
            }
        }
        self.bus.reset_mmio_activity_counters();
        let state = AdvanceState { fuel_consumed, start_cycles };
        let count = self.plan_advance_batch(request, &state);
        if count == 0 {
            return Ok(AdvanceReport::new(AdvanceStop::NoProgress, fuel_consumed,
                primary_steps, secondary_steps, self.total_cycles - start_cycles,
                idle_cycles, cpu_batches));
        }
        let batch_start = self.total_cycles;
        let published_cycle = if request.is_single() {
            batch_start + 1
        } else {
            batch_start
        };
        self.bus.set_current_cycle(published_cycle);
        self.bus.bus_trace.set_cycle(published_cycle);
        self.logic_seed_batch_clock(published_cycle);
        let progress = self.execute_cpu_window(count)?;
        if progress.primary_steps == 0 {
            return Ok(AdvanceReport::new(AdvanceStop::NoProgress, fuel_consumed,
                primary_steps, secondary_steps, self.total_cycles - start_cycles,
                idle_cycles, cpu_batches));
        }
        self.commit_advance_boundary(batch_start, progress)?;
        fuel_consumed += u64::from(progress.primary_steps);
        primary_steps += u64::from(progress.primary_steps);
        secondary_steps += u64::from(progress.secondary_steps);
        cpu_batches += 1;
    }
}
```

Define `AdvanceReport::new` as a crate-private constructor that assigns all seven fields. `commit_advance_boundary` owns the extracted counter/profile update, tick costs and primary IRQ delivery, post-cost bus clock, scheduler drain, RTC reset, SCB reset, flash operation, and logic observation. For `AdvanceRequest::single`, it preserves the legacy pre-instruction cycle by passing a one-step `CoreProgress` through the same boundary code. Do not duplicate any lifecycle operation in `advance.rs` and `boundary.rs`.

- [ ] **Step 5: Convert DebugControl adapters**

```rust
fn run(&mut self, max_steps: Option<u32>) -> SimResult<StopReason> {
    let report = Machine::advance(self, AdvanceRequest::run(max_steps.map(u64::from)))?;
    Ok(match report.stop {
        AdvanceStop::Breakpoint(pc) => StopReason::Breakpoint(pc),
        AdvanceStop::FuelLimit => StopReason::MaxStepsReached,
        AdvanceStop::CycleLimit | AdvanceStop::NoProgress => StopReason::StepDone,
    })
}

fn step_single(&mut self) -> SimResult<StopReason> {
    Machine::advance(self, AdvanceRequest::single())?;
    Ok(StopReason::StepDone)
}

fn reset(&mut self) -> SimResult<()> { Machine::reset(self) }
```

- [ ] **Step 6: Run core fidelity after this step**

```bash
cargo test -p labwired-core machine_advance -- --nocapture
cargo test -p labwired-core rtc_reset -- --nocapture
cargo test -p labwired-core scb_reset -- --nocapture
cargo test -p labwired-core --features event-scheduler --test systick_walk_differential --test stm32_timer_walk_differential --test stm32_dma_walk_differential --test esp32s3_walk_differential -- --nocapture
cargo test --release -p labwired-core --features jit,event-scheduler --test riscv_jit_c3_oled_differential jit_vs_interpreter_c3_oled_is_byte_identical_and_non_vacuous -- --exact --ignored --nocapture
```

Expected: PASS. Missing fixture-backed ignored gates are reported, never counted as passing.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/lib.rs crates/core/src/machine crates/core/src/tests/machine_advance.rs crates/core/src/tests/scb_reset.rs
git commit -m "refactor(core): unify batched machine execution"
```

## Task 5: Migrate the CLI Test Runner

**Files:**
- Modify: `crates/cli/src/main.rs`
- Create: `crates/cli/tests/machine_advance_fidelity.rs`

- [ ] **Step 1: Add CLI single-versus-batch artifact parity**

Before replacing the old loop, add private `ExecutionEngine { Legacy, Unified }` selection from the temporary `LABWIRED_TEST_EXECUTOR` environment variable. Default to `Legacy` until parity passes. Keep the current JIT/manual/single branches intact under `Legacy`; add the new `Machine::advance` branch under `Unified`. Emit exactly one stderr marker, `executor=legacy` or `executor=unified`, so the test proves both arms ran. The selector and legacy branch are removed only in Task 8.

Drive the real CLI twice with `tests/fixtures/uart-ok-thumbv7m.elf`, once with each engine. Build each script in a distinct temporary directory, invoke `env!("CARGO_BIN_EXE_labwired") test --script ... --no-uart-stdout --output-dir ...`, read `result.json`, `snapshot.json`, `junit.xml`, and `uart.log`, and compare all artifacts after removing only JUnit elapsed-time attributes:

```rust
#[test]
fn cli_unified_batch_matches_single_step_artifacts() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let firmware = root.join("tests/fixtures/uart-ok-thumbv7m.elf");
    let tmp = std::env::temp_dir().join(format!("lw-advance-fidelity-{}", std::process::id()));
    let legacy = run_fixture(&tmp.join("legacy"), &firmware, "legacy");
    let unified = run_fixture(&tmp.join("unified"), &firmware, "unified");
    assert!(legacy.exit_ok && unified.exit_ok, "legacy={} unified={}\n{}\n{}", legacy.exit_ok, unified.exit_ok, legacy.stderr, unified.stderr);
    assert!(legacy.stderr.contains("executor=legacy"));
    assert!(unified.stderr.contains("executor=unified"));
    assert_eq!(legacy.uart, unified.uart);
    assert_eq!(legacy.result, unified.result);
    assert_eq!(legacy.snapshot, unified.snapshot);
    assert_eq!(normalize_junit(&legacy.junit), normalize_junit(&unified.junit));
    std::fs::remove_dir_all(tmp).unwrap();
}
```

Define `RunOutput { result: Vec<u8>, snapshot: Vec<u8>, junit: String, uart: Vec<u8>, exit_ok: bool, stderr: String }`. `run_fixture` creates its directory, writes a script with `max_steps: 32` and `expected_stop_reason: max_steps`, sets `LABWIRED_TEST_EXECUTOR` to its engine argument, invokes the binary, and reads the four artifacts. `normalize_junit` replaces only numeric values of `time="..."` attributes with `time="0"`; it must not strip cycles, instructions, CPU state, assertions, logic edges, stop details, or any JSON fields.

- [ ] **Step 2: Run baseline parity**

```bash
cargo test -p labwired-cli --test machine_advance_fidelity -- --nocapture
```

Expected: baseline established; if manual batching differs, record fields and keep single-step as oracle.

- [ ] **Step 3: Replace both execution branches**

Implement the `Unified` arm without changing the retained `Legacy` arm. Keep host policies outside and use:

```rust
let mut limit = u64::from(to_execute);
let cur = machine.total_cycles;
for (stimulus, fired) in &pending_stimuli {
    if !*fired {
        if let labwired_config::FaultTrigger::AfterCycles { cycles } = stimulus.trigger {
            if cycles > cur { limit = limit.min(cycles - cur); }
        }
    }
}
if let Some(max_cycles) = max_cycles {
    if max_cycles > cur { limit = limit.min(max_cycles - cur); }
}
let request = AdvanceRequest::run(Some(limit.max(1)))
    .with_batch_cap(NonZeroU32::new(current_batch.max(1)).unwrap())
    .with_breakpoints(BreakpointPolicy::Ignore);
match machine.advance(request) {
    Ok(report) => {
        step += report.primary_steps;
        steps_executed = step;
        if report.primary_steps == 0 && report.idle_cycles == 0 { stop_reason = StopReason::Halt; break; }
    }
    Err(error) => { sim_error_happened = true; stop_reason = map_sim_error_to_stop_reason(&error); break; }
}
```

The `Unified` arm contains no manual clocks, ticks, scheduler work, logic observation, SCB reset, or profile bookkeeping; those remain temporarily only inside the test-selected `Legacy` arm.

- [ ] **Step 4: Run CLI fidelity at this step**

```bash
cargo test -p labwired-cli --test machine_advance_fidelity -- --nocapture
cargo test -p labwired-cli --test runner --test outputs --test snapshots --test interactive_snapshot --test determinism --test golden_examples --test stop_conditions -- --nocapture
cargo test --release -p labwired-cli --features jit-core --test riscv_jit_c3_oled_test_differential --test riscv_tick_interval_fidelity_differential -- --nocapture
```

Expected: exact normalized parity and PASS.

- [ ] **Step 5: Make unified execution the production default**

Change the absent/unknown `LABWIRED_TEST_EXECUTOR` case to `ExecutionEngine::Unified`, rerun `machine_advance_fidelity`, and verify both explicit legacy/unified markers still appear in their respective arms.

- [ ] **Step 6: Verify the ordinary direct CPU call is isolated**

```bash
rg -n "machine\.cpu\.step(_batch)?" crates/cli/src/main.rs
```

Expected: matches exist only inside the clearly delimited temporary `ExecutionEngine::Legacy` branch; the default/tested `Unified` arm has none.

- [ ] **Step 7: Commit**

```bash
git add crates/cli/src/main.rs crates/cli/tests/machine_advance_fidelity.rs
git commit -m "refactor(cli): use unified machine advance"
```

## Task 6: Migrate Ordinary WASM Wrappers

**Files:**
- Modify: `crates/wasm/src/lib.rs`

- [ ] **Step 1: Add wrapper equivalence and schema tests**

Construct identical in-memory ARM, RISC-V, and Xtensa simulators with 64 harmless instructions. Compare 32 `step_single` calls with `step_batch(32)` CPU/peripheral snapshots and cycles. Assert every current `WasmStepBatchProfile` JSON key remains present.

```rust
fn wrap_test_machine<C: Cpu + 'static>(cpu: C, mut bus: SystemBus, arch: Arch) -> WasmSimulator {
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);
    let uart_rx_bufs = bus.attach_uart_rx_source();
    WasmSimulator {
        machine: Some(Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus)),
        board_io: Vec::new(), uart_sink, uart_rx_bufs, arch, esp32_ipi: None,
        jit_browser_enabled: false, jit_browser_cache: None,
    }
}

fn assert_batch_matches_singles(factory: impl Fn() -> WasmSimulator) {
    let mut singles = factory();
    let mut batch = factory();
    for _ in 0..32 { singles.step_single().unwrap(); }
    assert_eq!(batch.step_batch(32).unwrap(), 32);
    assert_eq!(
        serde_json::to_value(batch.machine.as_ref().unwrap().snapshot()).unwrap(),
        serde_json::to_value(singles.machine.as_ref().unwrap().snapshot()).unwrap(),
    );
    assert_eq!(batch.machine.as_ref().unwrap().total_cycles, singles.machine.as_ref().unwrap().total_cycles);
}

#[test]
fn wasm_arm_batch_matches_repeated_single_steps() {
    assert_batch_matches_singles(|| {
        let mut bus = SystemBus::new();
        let (mut cpu, _) = configure_cortex_m(&mut bus);
        for pc in (0u64..128).step_by(2) { bus.write_u16(pc, 0xBF00).unwrap(); }
        cpu.set_pc(0);
        wrap_test_machine(cpu, bus, Arch::Arm)
    });
}

#[test]
fn wasm_riscv_batch_matches_repeated_single_steps() {
    assert_batch_matches_singles(|| {
        let mut bus = SystemBus::new();
        let mut cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
        for pc in (0u64..256).step_by(4) { bus.write_u32(pc, 0x0000_0013).unwrap(); }
        cpu.set_pc(0);
        wrap_test_machine(cpu, bus, Arch::RiscV)
    });
}

#[test]
fn wasm_xtensa_batch_matches_repeated_single_steps() {
    assert_batch_matches_singles(|| {
        let mut bus = SystemBus::new();
        let mut cpu = labwired_core::cpu::xtensa_lx7::XtensaLx7::new();
        for pc in (0u64..128).step_by(2) {
            bus.write_u8(pc, 0x3d).unwrap();
            bus.write_u8(pc + 1, 0xf0).unwrap();
        }
        cpu.set_pc(0);
        wrap_test_machine(cpu, bus, Arch::Xtensa)
    });
}

#[test]
fn wasm_step_batch_profile_keeps_existing_json_keys() {
    let value = serde_json::to_value(WasmStepBatchProfile {
        requested_cycles: 1, executed_cycles: 1, wall_ms: 0.0, cycles_per_second: 0.0,
        cpu_instructions: 1, cpu_batches: 1, peripheral_ticks: 1,
        peripheral_ticked_entries: 0, bus_tick_entries: 0, legacy_tick_entries: 0,
    }).unwrap();
    for key in ["requested_cycles", "executed_cycles", "wall_ms", "cycles_per_second",
        "cpu_instructions", "cpu_batches", "peripheral_ticks",
        "peripheral_ticked_entries", "bus_tick_entries", "legacy_tick_entries"] {
        assert!(value.get(key).is_some(), "missing profile key {key}");
    }
}
```

- [ ] **Step 2: Run baseline**

```bash
cargo test -p labwired-wasm wasm_arm_batch_matches_repeated_single_steps -- --nocapture
cargo test -p labwired-wasm wasm_riscv_batch_matches_repeated_single_steps -- --nocapture
cargo test -p labwired-wasm wasm_xtensa_batch_matches_repeated_single_steps -- --nocapture
```

Expected: baseline established; repeated single-step is oracle for divergence.

- [ ] **Step 3: Migrate step, step_single, step_batch, and profile**

Preserve the JS return as total-cycle delta:

```rust
pub fn step_batch(&mut self, max_cycles: u32) -> Result<u32, JsValue> {
    let machine = self.machine();
    let before = machine.total_cycles;
    let result = machine.advance(AdvanceRequest::run(Some(u64::from(max_cycles))));
    let executed = (machine.total_cycles - before) as u32;
    match result {
        Ok(_) => Ok(executed),
        Err(_) if executed > 0 => Ok(executed),
        Err(error) => Err(JsValue::from_str(&format!("Step Error: {error}"))),
    }
}
```

Use the successful report's `elapsed_cycles` for `executed_cycles`; on error preserve the old partial-progress behavior by computing `machine.total_cycles - before`. Serialize the existing shape exactly:

```rust
let advance_result = machine.advance(AdvanceRequest::run(Some(u64::from(max_cycles))));
let executed = match &advance_result {
    Ok(report) => report.elapsed_cycles.min(u64::from(u32::MAX)) as u32,
    Err(_) => (machine.total_cycles - before).min(u64::from(u32::MAX)) as u32,
};
let profile = machine.step_profile();
if let Err(error) = advance_result {
    if executed == 0 {
        return Err(JsValue::from_str(&format!("Step Error: {error}")));
    }
}
serde_wasm_bindgen::to_value(&WasmStepBatchProfile {
    requested_cycles: max_cycles,
    executed_cycles: executed,
    wall_ms: t1 - t0,
    cycles_per_second: if t1 > t0 { f64::from(executed) * 1000.0 / (t1 - t0) } else { 0.0 },
    cpu_instructions: profile.cpu_instructions,
    cpu_batches: profile.cpu_batches,
    peripheral_ticks: profile.peripheral_ticks,
    peripheral_ticked_entries: profile.peripheral_ticked_entries,
    bus_tick_entries: profile.bus_tick_entries,
    legacy_tick_entries: profile.legacy_tick_entries,
})
```

Leave `step_with_esp32_aids` on the single-step adapter and do not enable or move browser JIT.

- [ ] **Step 4: Run WASM fidelity**

```bash
cargo test -p labwired-wasm
cargo check -p labwired-wasm --target wasm32-unknown-unknown
```

Expected: PASS and wasm32 check exit zero.

- [ ] **Step 5: Commit**

```bash
git add crates/wasm/src/lib.rs
git commit -m "refactor(wasm): use unified machine advance"
```

## Task 7: Document the Boundary and Inventory Remaining Paths

**Files:**
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/tests/machine_advance.rs`
- Modify: `crates/cli/src/main.rs`
- Modify: `docs/architecture.md`

- [ ] **Step 1: Inventory remaining direct stepping**

```bash
rg -n "\.cpu\.step(_batch)?\(|cpu\.step(_batch)?\(" crates --glob '*.rs' -g '!**/tests/**' -g '!**/*test*.rs'
```

Allowed: the temporary `ExecutionEngine::Legacy` CLI fidelity branch, hardware-oracle bare-CPU code, specialized CLI snapshot/ESP loops listed as non-goals, and CPU/peripheral unit helpers. Forbidden: the production/default CLI branch, WASM ordinary wrappers, and `DebugControl::run`.

- [ ] **Step 2: Document the lifecycle**

Add:

```markdown
### Authoritative machine advancement

Ordinary native, CLI-test, debugger, and WASM execution enters through
`Machine::advance`. It owns batch planning, CPU0/CPU1 scheduling, cycle
publication, peripheral ticks, scheduler delivery, software reset, flash
operations, profiling, and logic observation. Frontends inspect the machine
between bounded calls for assertions, stimuli, UART limits, and artifacts, but
must not call `Cpu::step` directly.
```

- [ ] **Step 3: Run documentation-boundary fidelity**

```bash
cargo test -p labwired-core machine_advance -- --nocapture
cargo test -p labwired-core scb_reset -- --nocapture
cargo test -p labwired-core logic_capture -- --nocapture
cargo test -p labwired-cli --test machine_advance_fidelity -- --nocapture
cargo test -p labwired-wasm machine_advance_tests -- --nocapture
git diff --check
```

Expected: PASS; diff check empty.

- [ ] **Step 4: Commit**

```bash
git add crates/core/src/lib.rs crates/core/src/tests/machine_advance.rs crates/cli/src/main.rs docs/architecture.md
git commit -m "docs(core): establish authoritative advance lifecycle"
```

## Task 8: Final Fidelity and Quality Gate

**Files:**
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/tests/machine_advance.rs`
- Modify: `crates/cli/src/main.rs`
- Delete: `crates/cli/tests/machine_advance_fidelity.rs`

- [ ] **Step 1: Run core suites**

```bash
cargo test -p labwired-core
cargo test -p labwired-core --features event-scheduler
cargo test -p labwired-core --features jit,event-scheduler
```

Expected: all non-ignored tests pass.

- [ ] **Step 2: Run architecture differentials**

```bash
cargo test -p labwired-core --features event-scheduler --test systick_walk_differential --test stm32_timer_walk_differential --test stm32_dma_walk_differential --test esp32s3_walk_differential -- --nocapture
cargo test --release -p labwired-core --features jit,event-scheduler --test riscv_jit_c3_oled_differential jit_vs_interpreter_c3_oled_is_byte_identical_and_non_vacuous -- --exact --ignored --nocapture
```

Expected: PASS with nonzero JIT-path evidence.

- [ ] **Step 3: Run fixture-backed ignored fidelity where assets exist**

```bash
cargo test --release -p labwired-core --features event-scheduler --test esp32c3_walk_differential -- --ignored --nocapture
cargo test --release -p labwired-core --features event-scheduler --test esp32c3_clamped_full_state_differential oled_lab_full_state_byte_identical_interval_1_vs_64 -- --ignored --nocapture
cargo test --release -p labwired-core --test e2e_esp32c3_snapshot_resume -- --ignored --nocapture
```

Expected: PASS when documented firmware assets exist; otherwise report exact missing asset/error as not run.

- [ ] **Step 4: Run CLI and WASM suites**

```bash
cargo test -p labwired-cli
cargo test -p labwired-cli --features jit-core
cargo test -p labwired-wasm
cargo check -p labwired-wasm --target wasm32-unknown-unknown
```

Expected: PASS.

- [ ] **Step 5: Format and lint**

```bash
cargo fmt --all -- --check
cargo clippy -p labwired-core -p labwired-cli -p labwired-wasm --all-targets -- -D warnings
git diff --check
```

Expected: zero exit status and empty diff check.

- [ ] **Step 6: Remove the temporary comparison executors**

Delete `Machine::step_legacy_for_test`, obsolete tests that assert the intentionally old run omissions, the CLI `ExecutionEngine` selector, `LABWIRED_TEST_EXECUTOR` handling/markers, the retained legacy CLI branch, and the now-temporary `machine_advance_fidelity.rs` integration test. Keep the permanent public-behavior tests for CPU1 run, RTC reset, single-step accounting, breakpoints, and WASM wrapper parity.

```bash
cargo test -p labwired-core machine_advance -- --nocapture
cargo test -p labwired-core scb_reset -- --nocapture
cargo test -p labwired-cli --test runner --test outputs --test snapshots --test determinism -- --nocapture
cargo test -p labwired-wasm machine_advance_tests -- --nocapture
rg -n "LABWIRED_TEST_EXECUTOR|ExecutionEngine|step_legacy_for_test|run_legacy_for_test|TEMPORARY LEGACY TEST EXECUTOR|executor=(legacy|unified)" crates
```

Expected: tests PASS and the final `rg` has no matches.

- [ ] **Step 7: Commit legacy removal**

```bash
git add crates/core/src/lib.rs crates/core/src/tests/machine_advance.rs crates/cli/src/main.rs crates/cli/tests/machine_advance_fidelity.rs
git commit -m "refactor(core): remove legacy execution loops"
```

- [ ] **Step 8: Run repository-standard final verification**

```bash
cargo check --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture
cargo test --workspace --lib
cargo test --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture
cargo clippy -p labwired-core -p labwired-cli -p labwired-wasm --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Expected: all commands exit zero.

- [ ] **Step 9: Verify success criteria**

```bash
rg -n "machine\.cpu\.step(_batch)?" crates/cli/src/main.rs crates/wasm/src/lib.rs
rg -n "fn advance\(|Machine::advance|\.advance\(" crates/core/src crates/cli/src/main.rs crates/wasm/src/lib.rs
git status --short --branch
```

Expected: ordinary surfaces use advance; only documented specialized callers remain; pre-existing `.claude/` is untouched.

- [ ] **Step 10: Confirm the verification run introduced no edits**

```bash
git status --short
```

Expected: only the pre-existing `.claude/` entry is untracked; verification itself changed no tracked files.
