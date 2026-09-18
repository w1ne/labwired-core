// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Architectural WFE/SEV behavior through the public CPU interface.
use labwired_core::{
    bus::SystemBus,
    cpu::CortexM,
    decoder::arm::{decode_thumb_16, decode_thumb_32, Instruction},
    Bus, Cpu, DmaRequest, SimResult, SimulationConfig,
};
use std::{collections::HashMap, sync::atomic::Ordering};
struct MockBus {
    mem: HashMap<u64, u8>,
    config: SimulationConfig,
}

impl MockBus {
    fn new() -> Self {
        Self {
            mem: HashMap::new(),
            config: SimulationConfig::default(),
        }
    }
}

impl Bus for MockBus {
    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        Ok(*self.mem.get(&addr).unwrap_or(&0))
    }
    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
        self.mem.insert(addr, value);
        Ok(())
    }
    fn tick_peripherals(&mut self) -> Vec<u32> {
        Vec::new()
    }
    fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
        Ok(())
    }
    fn config(&self) -> &SimulationConfig {
        &self.config
    }
}

fn run_test_instr(cpu: &mut CortexM, bus: &mut MockBus, instr_bin: u32, is_32bit: bool) {
    let pc = cpu.pc;
    if is_32bit {
        bus.write_u16(pc as u64, (instr_bin >> 16) as u16).unwrap();
        bus.write_u16((pc + 2) as u64, (instr_bin & 0xFFFF) as u16)
            .unwrap();
    } else {
        bus.write_u16(pc as u64, instr_bin as u16).unwrap();
    }
    let config = bus.config.clone();
    cpu.step(bus, &[], &config).unwrap();
}

#[test]
fn wfe_parks_until_event_and_sev_is_consumed_once() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    run_test_instr(&mut cpu, &mut bus, 0xBF40, false); // SEV
    run_test_instr(&mut cpu, &mut bus, 0xBF20, false); // consumes event
    assert!(cpu.idle_fast_forward_budget(&bus).is_none());
    run_test_instr(&mut cpu, &mut bus, 0xBF20, false); // now sleeps
    assert!(cpu.idle_fast_forward_budget(&bus).is_some());
    let parked_pc = cpu.pc;
    run_test_instr(&mut cpu, &mut bus, 0x3001, false); // ADD must not execute
    assert_eq!(cpu.pc, parked_pc);
    assert_eq!(cpu.r0, 0);
}

#[test]
fn wfe_does_not_wake_for_primask_masked_interrupt() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.primask = true;
    run_test_instr(&mut cpu, &mut bus, 0xBF20, false);
    cpu.set_exception_pending(15);
    assert!(cpu.idle_fast_forward_budget(&bus).is_some());
    cpu.primask = false;
    assert!(cpu.idle_fast_forward_budget(&bus).is_none());
}

#[test]
fn wfe_sevonpend_wakes_on_disabled_irq_transition_only() {
    let mut bus = SystemBus::new();
    let (mut cpu, nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    bus.write_u16(0, 0xBF20).unwrap();
    bus.write_u16(2, 0xBF20).unwrap();
    bus.write_u16(4, 0xBF20).unwrap();
    bus.write_u32(0xE000_ED10, 1 << 4).unwrap();
    let cfg = bus.config.clone();
    cpu.step(&mut bus, &[], &cfg).unwrap();
    assert!(cpu.idle_fast_forward_budget(&bus).is_some());
    // ISER is zero: a disabled external interrupt still sets the event latch.
    bus.write_u32(0xE000_E200, 1 << 5).unwrap();
    assert!(cpu.idle_fast_forward_budget(&bus).is_none());
    cpu.step(&mut bus, &[], &cfg).unwrap();
    assert_eq!(cpu.pc, 4);
    assert!(cpu.idle_fast_forward_budget(&bus).is_some());
    // An already-pending line is not a fresh event.
    bus.write_u32(0xE000_E200, 1 << 5).unwrap();
    assert!(cpu.idle_fast_forward_budget(&bus).is_some());
    assert_eq!(nvic.iser[0].load(Ordering::Relaxed), 0);
    bus.write_u32(0xE000_E280, 1 << 5).unwrap();
    bus.write_u32(0xE000_E200, 1 << 5).unwrap();
    assert!(cpu.idle_fast_forward_budget(&bus).is_none());
}

#[test]
fn wfe_exception_return_records_event_before_thread_sleeps() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    cpu.pc = 0x1000;
    cpu.sp = 0x8000;
    bus.write_u32(15 * 4, 0x5001).unwrap();
    bus.write_u16(0x5000, 0x4770).unwrap(); // BX LR
    bus.write_u16(0x1000, 0xBF20).unwrap();
    bus.write_u16(0x1002, 0xBF20).unwrap();
    let cfg = bus.config.clone();
    cpu.set_exception_pending(15);
    cpu.step(&mut bus, &[], &cfg).unwrap();
    cpu.step(&mut bus, &[], &cfg).unwrap(); // exception return
    assert_eq!(cpu.pc, 0x1000);
    cpu.step(&mut bus, &[], &cfg).unwrap(); // consumes return event
    assert!(cpu.idle_fast_forward_budget(&bus).is_none());
    cpu.step(&mut bus, &[], &cfg).unwrap(); // sleeps
    assert!(cpu.idle_fast_forward_budget(&bus).is_some());
}

#[test]
fn wfe_wide_hints_match_narrow_encodings() {
    assert_eq!(decode_thumb_32(0xF3AF, 0x8002), decode_thumb_16(0xBF20));
    assert_eq!(decode_thumb_32(0xF3AF, 0x8003), Instruction::Wfi);
    assert_eq!(decode_thumb_32(0xF3AF, 0x8004), decode_thumb_16(0xBF40));
}

#[test]
fn wfe_ignores_cleared_or_disabled_external_pending_bits() {
    let mut bus = SystemBus::new();
    let (mut cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    bus.write_u16(0, 0xBF20).unwrap();
    bus.write_u32(0xE000_E100, 1).unwrap();
    let cfg = bus.config.clone();
    cpu.step(&mut bus, &[], &cfg).unwrap();
    bus.write_u32(0xE000_E200, 1).unwrap();
    cpu.set_exception_pending(16);
    assert!(cpu.idle_fast_forward_budget(&bus).is_none());
    bus.write_u32(0xE000_E280, 1).unwrap();
    assert!(cpu.idle_fast_forward_budget(&bus).is_some());
    bus.write_u32(0xE000_E200, 1).unwrap();
    bus.write_u32(0xE000_E180, 1).unwrap();
    assert!(cpu.idle_fast_forward_budget(&bus).is_some());
    cpu.step(&mut bus, &[], &cfg).unwrap();
    assert_eq!(cpu.pc, 2);
}

#[test]
fn wfe_snapshot_preserves_sleep_and_unconsumed_event() {
    let mut cpu = CortexM::new();
    let mut bus = MockBus::new();
    run_test_instr(&mut cpu, &mut bus, 0xBF20, false);
    let mut restored = CortexM::new();
    restored.apply_snapshot(&cpu.snapshot());
    assert!(restored.idle_fast_forward_budget(&bus).is_some());
    let mut signaled = CortexM::new();
    run_test_instr(&mut signaled, &mut bus, 0xBF40, false);
    restored.apply_snapshot(&signaled.snapshot());
    run_test_instr(&mut restored, &mut bus, 0xBF20, false);
    assert!(restored.idle_fast_forward_budget(&bus).is_none());
    run_test_instr(&mut restored, &mut bus, 0xBF20, false);
    assert!(restored.idle_fast_forward_budget(&bus).is_some());
}
