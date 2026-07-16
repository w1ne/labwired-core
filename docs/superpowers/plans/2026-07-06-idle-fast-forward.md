# Idle Fast-Forward Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a switchable idle fast-forward path that accelerates RISC-V WFI waits without changing default emulator behavior.

**Architecture:** Add a `SimulationConfig` boolean that defaults off. RISC-V marks itself as waiting after WFI, and the `Machine::run` loop fast-forwards only when that flag is set, no breakpoint/cycle-accurate service is active, the scheduler-backed bus can safely skip legacy ticks, and no interrupt is already pending.

**Tech Stack:** Rust core crate, existing `SimulationConfig`, `Cpu` trait, `Machine::run`, and event-scheduler hooks.

---

### Task 1: Switchable WFI Fast-Forward Contract

**Files:**
- Modify: `crates/core/src/config.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/cpu/riscv.rs`

- [ ] **Step 1: Write the failing tests**

Add tests in `crates/core/src/cpu/riscv.rs`:

```rust
#[test]
fn test_riscv_wfi_fast_forward_is_off_by_default() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    bus.flash.data = vec![0; 0x100];
    bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
    bus.write_u32(0x4, 0x0000_006f).unwrap(); // JAL x0, 0
    cpu.pc = 0x0;
    let mut machine = Machine::new(cpu, bus);
    machine.bus.legacy_walk_disabled = true;
    machine.run(Some(10)).unwrap();
    assert_eq!(machine.total_cycles, 10);
    assert_eq!(machine.step_profile().cpu_instructions, 10);
}
```

```rust
#[test]
fn test_riscv_wfi_fast_forward_skips_cpu_work_when_enabled() {
    let mut bus = SystemBus::new();
    let mut cpu = RiscV::new();
    bus.flash.data = vec![0; 0x100];
    bus.write_u32(0x0, 0x1050_0073).unwrap(); // WFI
    bus.write_u32(0x4, 0x0000_006f).unwrap(); // JAL x0, 0
    cpu.pc = 0x0;
    let mut machine = Machine::new(cpu, bus);
    machine.config.idle_fast_forward_enabled = true;
    machine.bus.legacy_walk_disabled = true;
    machine.run(Some(10)).unwrap();
    assert_eq!(machine.total_cycles, 10);
    assert!(machine.step_profile().cpu_instructions < 10);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p labwired-core test_riscv_wfi_fast_forward --features event-scheduler`

Expected: FAIL because `idle_fast_forward_enabled` and CPU waiting-state hooks do not exist yet.

- [ ] **Step 3: Add minimal switch and CPU hook**

Add `idle_fast_forward_enabled: bool` to `SimulationConfig`, default `false`. Add CPU trait hooks for idle waiting and fast-forwarding elapsed cycles. Implement them only for `RiscV`, setting the waiting flag on `Instruction::Wfi` and advancing `mtime`/`mip` during skipped cycles.

- [ ] **Step 4: Add minimal run-loop skip**

At the top of `Machine::run`, if enabled and safe, advance by the smaller of remaining requested steps and next scheduler deadline or batch limit. Update `total_cycles`, `steps`, and scheduler time; leave `cpu_instructions` unchanged for skipped cycles.

- [ ] **Step 5: Run focused tests**

Run: `cargo test -p labwired-core test_riscv_wfi_fast_forward --features event-scheduler`

Expected: PASS.

### Task 2: Regression Coverage

**Files:**
- Modify: `crates/core/src/cpu/riscv.rs`
- Modify: `crates/core/src/config.rs`

- [ ] **Step 1: Confirm existing WFI behavior still passes**

Run: `cargo test -p labwired-core test_riscv_wfi_is_nop`

Expected: PASS.

- [ ] **Step 2: Confirm default config is unchanged except explicit field**

Add a small assertion that `SimulationConfig::default().idle_fast_forward_enabled` is false.

- [ ] **Step 3: Run core tests touched by this path**

Run: `cargo test -p labwired-core test_riscv_timer_interrupt test_riscv_wfi --features event-scheduler`

Expected: PASS.
