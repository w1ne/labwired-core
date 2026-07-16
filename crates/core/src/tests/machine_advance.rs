//! Permanent boundary and adapter tests for the authoritative
//! `Machine::advance` lifecycle.
//!
//! Production callers enter through `Machine::advance` or its public adapters.

use crate::bus::SystemBus;
use crate::runtime_snapshot::CpuKind;
use crate::snapshot::{ArmCpuSnapshot, CpuSnapshot};
use crate::{
    AdvanceReport, AdvanceRequest, AdvanceStop, BatchPolicy, BreakpointPolicy, Bus, Cpu,
    DebugControl, IdlePolicy, Machine, SimResult, SimulationConfig, SimulationError,
    SimulationObserver, StepProfile, StopReason,
};
use std::num::NonZeroU32;
use std::sync::Arc;

#[derive(Debug, Default)]
struct CountingCpu {
    pc: u32,
    sp: u32,
    steps: u32,
    pending: Vec<u64>,
    halted: bool,
    // Non-architectural test injection; intentionally omitted from snapshots.
    fail_step: bool,
    // Non-architectural push-capture injection; intentionally omitted from snapshots.
    push_level: Option<bool>,
    // Non-architectural execution probes; intentionally omitted from snapshots.
    zero_batch: bool,
    fail_batch_after: Option<u32>,
    idle_budget: Option<u64>,
    idle_skipped: u64,
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

    fn step(
        &mut self,
        bus: &mut dyn Bus,
        _observers: &[Arc<dyn SimulationObserver>],
        _config: &SimulationConfig,
    ) -> SimResult<()> {
        if self.fail_step {
            return Err(SimulationError::Other(
                "CountingCpu injected step failure".to_string(),
            ));
        }
        if let Some(level) = self.push_level {
            bus.logic_tap().unwrap().push(0, level);
        }
        if !self.halted {
            self.steps += 1;
            self.pc = self.pc.wrapping_add(2);
        }
        Ok(())
    }

    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        if self.zero_batch {
            return Ok(0);
        }
        let tap = bus.logic_tap().filter(|tap| tap.push_armed());
        for i in 0..max_count {
            if self.fail_batch_after == Some(i) {
                return Err(SimulationError::Other(
                    "CountingCpu injected batch failure".to_string(),
                ));
            }
            if let Some(tap) = &tap {
                tap.bump_clock();
            }
            self.step(bus, observers, config)?;
            if config.idle_fast_forward_enabled && self.idle_fast_forward_budget(bus).is_some() {
                return Ok(i + 1);
            }
        }
        Ok(max_count)
    }

    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }

    fn get_pc(&self) -> u32 {
        self.pc
    }

    fn set_sp(&mut self, val: u32) {
        self.sp = val;
    }

    fn set_exception_pending(&mut self, exception_num: u32) {
        let word = exception_num as usize / 64;
        let bit = exception_num % 64;
        if self.pending.len() <= word {
            self.pending.resize(word + 1, 0);
        }
        self.pending[word] |= 1_u64 << bit;
    }

    fn get_register(&self, id: u8) -> u32 {
        match id {
            0 => self.steps,
            13 => self.sp,
            15 => self.pc,
            _ => 0,
        }
    }

    fn set_register(&mut self, id: u8, val: u32) {
        match id {
            0 => self.steps = val,
            13 => self.sp = val,
            15 => self.pc = val,
            _ => {}
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        let mut registers = vec![0; 16];
        registers[0] = self.steps;
        registers[13] = self.sp;
        registers[15] = self.pc;
        CpuSnapshot::Arm(ArmCpuSnapshot {
            registers,
            pc: self.pc,
            xpsr: 0,
            primask: false,
            pending_exceptions: self.pending.first().copied().unwrap_or(0),
            pending_exceptions_hi: self.pending.iter().skip(1).copied().collect(),
            vtor: 0,
        })
    }

    fn apply_snapshot(&mut self, snapshot: &CpuSnapshot) {
        if let CpuSnapshot::Arm(snapshot) = snapshot {
            self.steps = snapshot.registers.first().copied().unwrap_or(0);
            self.sp = snapshot.registers.get(13).copied().unwrap_or(0);
            self.pc = snapshot.pc;
            self.pending.clear();
            self.pending.push(snapshot.pending_exceptions);
            self.pending
                .extend(snapshot.pending_exceptions_hi.iter().copied());
        }
    }

    fn runtime_snapshot(&self) -> (CpuKind, Vec<u8>) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.pc.to_le_bytes());
        bytes.extend_from_slice(&self.sp.to_le_bytes());
        bytes.extend_from_slice(&self.steps.to_le_bytes());
        bytes.push(u8::from(self.halted));
        bytes.extend_from_slice(&(self.pending.len() as u64).to_le_bytes());
        for word in &self.pending {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        (CpuKind::ArmCortexM, bytes)
    }

    fn apply_runtime_snapshot(&mut self, kind: CpuKind, bytes: &[u8]) -> SimResult<()> {
        if kind != CpuKind::ArmCortexM {
            return Err(SimulationError::NotImplemented(format!(
                "CountingCpu runtime snapshot has wrong CPU kind: {kind:?}"
            )));
        }

        const HEADER_LEN: usize = 21;
        if bytes.len() < HEADER_LEN {
            return Err(SimulationError::NotImplemented(format!(
                "CountingCpu runtime snapshot is truncated: {} bytes",
                bytes.len()
            )));
        }

        let read_u32 = |offset: usize| {
            let mut raw = [0_u8; 4];
            raw.copy_from_slice(&bytes[offset..offset + 4]);
            u32::from_le_bytes(raw)
        };
        let mut pending_len_raw = [0_u8; 8];
        pending_len_raw.copy_from_slice(&bytes[13..21]);
        let pending_len = usize::try_from(u64::from_le_bytes(pending_len_raw)).map_err(|_| {
            SimulationError::NotImplemented(
                "CountingCpu runtime snapshot pending length does not fit usize".to_string(),
            )
        })?;
        let expected_len = pending_len
            .checked_mul(8)
            .and_then(|pending_bytes| HEADER_LEN.checked_add(pending_bytes))
            .ok_or_else(|| {
                SimulationError::NotImplemented(
                    "CountingCpu runtime snapshot length overflow".to_string(),
                )
            })?;
        if bytes.len() != expected_len {
            return Err(SimulationError::NotImplemented(format!(
                "CountingCpu runtime snapshot length mismatch: expected {expected_len}, got {}",
                bytes.len()
            )));
        }
        let halted = match bytes[12] {
            0 => false,
            1 => true,
            value => {
                return Err(SimulationError::NotImplemented(format!(
                    "CountingCpu runtime snapshot has invalid halted flag: {value}"
                )));
            }
        };
        let mut pending = Vec::with_capacity(pending_len);
        for chunk in bytes[HEADER_LEN..].chunks_exact(8) {
            let mut raw = [0_u8; 8];
            raw.copy_from_slice(chunk);
            pending.push(u64::from_le_bytes(raw));
        }

        self.pc = read_u32(0);
        self.sp = read_u32(4);
        self.steps = read_u32(8);
        self.pending = pending;
        self.halted = halted;
        Ok(())
    }

    fn get_register_names(&self) -> Vec<String> {
        (0..=12)
            .map(|id| format!("R{id}"))
            .chain(["SP", "LR", "PC"].into_iter().map(String::from))
            .collect()
    }

    fn index_of_register(&self, name: &str) -> Option<u8> {
        if name.eq_ignore_ascii_case("SP") {
            return Some(13);
        }
        if name.eq_ignore_ascii_case("LR") {
            return Some(14);
        }
        if name.eq_ignore_ascii_case("PC") {
            return Some(15);
        }
        let id = name
            .strip_prefix('R')
            .or_else(|| name.strip_prefix('r'))?
            .parse::<u8>()
            .ok()?;
        (id <= 12).then_some(id)
    }

    fn halt(&mut self) {
        self.halted = true;
    }

    fn unhalt(&mut self) {
        self.halted = false;
    }

    fn idle_fast_forward_budget(&self, _bus: &dyn Bus) -> Option<u64> {
        self.idle_budget
    }

    fn fast_forward_idle_cycles(&mut self, cycles: u64) {
        self.idle_skipped += cycles;
    }
}

fn counting_dual_core_machine() -> Machine<CountingCpu> {
    Machine::new(CountingCpu::default(), crate::bus::SystemBus::new())
        .with_secondary_cpu(CountingCpu::default())
}

#[test]
fn step_adapter_advances_both_cores_once() {
    let mut machine = counting_dual_core_machine();

    machine.step().expect("step should succeed");

    assert_eq!(machine.cpu.steps, 1);
    assert_eq!(machine.cpu_secondary.as_ref().map(|cpu| cpu.steps), Some(1));
    assert_eq!(machine.total_cycles, 1);
}

#[test]
fn step_adapter_publishes_and_profiles_one_cycle() {
    let mut machine = Machine::new(CountingCpu::default(), crate::bus::SystemBus::new());
    machine.reset_step_profile();

    machine.step().expect("step should succeed");

    assert_eq!(machine.total_cycles, 1);
    assert_eq!(machine.bus.current_cycle, 1);
    let profile = machine.step_profile();
    assert_eq!(profile.cpu_instructions, 1);
    assert_eq!(profile.cpu_batches, 1);
}

#[test]
fn reset_step_profile_clears_dirty_counters() {
    let mut machine = Machine::new(CountingCpu::default(), crate::bus::SystemBus::new());
    machine.step().expect("step should succeed");
    assert_ne!(machine.step_profile(), StepProfile::default());

    machine.reset_step_profile();

    assert_eq!(machine.step_profile(), StepProfile::default());
}

#[test]
fn counting_cpu_runtime_snapshot_round_trips() {
    let source = CountingCpu {
        pc: 0x1234_5678,
        sp: 0x2000_0100,
        steps: 42,
        pending: vec![0x8000_0000_0000_0001, 0x55AA],
        halted: true,
        fail_step: false,
        push_level: None,
        ..Default::default()
    };
    let (kind, bytes) = source.runtime_snapshot();
    let mut restored = CountingCpu::default();

    restored
        .apply_runtime_snapshot(kind, &bytes)
        .expect("valid CountingCpu runtime snapshot should restore");

    assert_eq!(restored.pc, source.pc);
    assert_eq!(restored.sp, source.sp);
    assert_eq!(restored.steps, source.steps);
    assert_eq!(restored.pending, source.pending);
    assert_eq!(restored.halted, source.halted);
}

#[test]
fn counting_cpu_runtime_snapshot_rejects_malformed_or_wrong_kind() {
    let mut cpu = CountingCpu::default();

    assert!(matches!(
        cpu.apply_runtime_snapshot(CpuKind::ArmCortexM, &[0; 3]),
        Err(SimulationError::NotImplemented(_))
    ));
    assert!(matches!(
        cpu.apply_runtime_snapshot(CpuKind::RiscV, &[]),
        Err(SimulationError::NotImplemented(_))
    ));
}

#[test]
fn counting_cpu_register_names_match_cortex_m() {
    let cpu = CountingCpu::default();
    let expected: Vec<String> = (0..=12)
        .map(|id| format!("R{id}"))
        .chain(["SP", "LR", "PC"].into_iter().map(String::from))
        .collect();

    assert_eq!(cpu.get_register_names(), expected);
    assert_eq!(cpu.index_of_register("r0"), Some(0));
    assert_eq!(cpu.index_of_register("R12"), Some(12));
    assert_eq!(cpu.index_of_register("sp"), Some(13));
    assert_eq!(cpu.index_of_register("Lr"), Some(14));
    assert_eq!(cpu.index_of_register("pc"), Some(15));
    assert_eq!(cpu.index_of_register("R13"), None);
}

#[test]
fn step_adapter_with_halted_primary_still_consumes_one_scheduling_quantum() {
    let mut machine = counting_dual_core_machine();
    machine.cpu.halt();

    machine.step().expect("step should succeed");

    assert_eq!(machine.cpu.steps, 0);
    assert_eq!(machine.cpu_secondary.as_ref().map(|cpu| cpu.steps), Some(1));
    assert_eq!(machine.total_cycles, 1);
    assert_eq!(machine.step_profile().cpu_instructions, 1);
}

#[test]
fn single_request_is_one_non_batched_non_idle_quantum() {
    let request = AdvanceRequest::single();

    assert_eq!(request.limits().fuel, Some(1));
    assert_eq!(request.limits().simulated_cycles, None);
    assert_eq!(request.breakpoint_policy(), BreakpointPolicy::Ignore);
    assert_eq!(request.idle_policy(), IdlePolicy::Disabled);
    assert_eq!(
        request.batch_policy(),
        BatchPolicy::AtMost(NonZeroU32::new(1).unwrap())
    );
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
    assert!(
        request.is_single(),
        "builders must preserve boundary timing mode"
    );
}

#[test]
fn advance_report_constructor_assigns_every_field() {
    let report = AdvanceReport::new(AdvanceStop::NoProgress, 1, 2, 3, 4, 5, 6);

    assert_eq!(report.stop, AdvanceStop::NoProgress);
    assert_eq!(report.fuel_consumed, 1);
    assert_eq!(report.primary_steps, 2);
    assert_eq!(report.secondary_steps, 3);
    assert_eq!(report.elapsed_cycles, 4);
    assert_eq!(report.idle_cycles, 5);
    assert_eq!(report.cpu_batches, 6);
}

#[test]
fn run_advances_in_capped_batches_and_reports_progress() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new());
    machine.config.peripheral_tick_interval = 64;

    let report = machine
        .advance(AdvanceRequest::run(Some(7)).with_batch_cap(NonZeroU32::new(3).unwrap()))
        .unwrap();

    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!(report.fuel_consumed, 7);
    assert_eq!(report.primary_steps, 7);
    assert_eq!(report.secondary_steps, 0);
    assert_eq!(report.elapsed_cycles, 7);
    assert_eq!(report.idle_cycles, 0);
    assert_eq!(report.cpu_batches, 3);
    assert_eq!(machine.cpu.steps, 7);
    assert_eq!(machine.step_profile().cpu_instructions, 7);
    assert_eq!(machine.step_profile().cpu_batches, 3);
}

#[test]
fn unified_run_advances_both_cores_one_quantum_at_a_time() {
    let mut machine = counting_dual_core_machine();
    machine.config.peripheral_tick_interval = 64;

    let report = machine.advance(AdvanceRequest::run(Some(4))).unwrap();

    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!((report.primary_steps, report.secondary_steps), (4, 4));
    assert_eq!((report.fuel_consumed, report.elapsed_cycles), (4, 4));
    assert_eq!(report.cpu_batches, 4);
    assert_eq!(machine.cpu.steps, 4);
    assert_eq!(machine.cpu_secondary.as_ref().unwrap().steps, 4);
    assert_eq!(machine.step_profile().cpu_instructions, 4);
    assert_eq!(machine.step_profile().cpu_batches, 4);
}

#[test]
fn debug_run_adapter_uses_unified_dual_core_execution() {
    let mut machine = counting_dual_core_machine();

    assert_eq!(machine.run(Some(4)).unwrap(), StopReason::MaxStepsReached);
    assert_eq!(machine.cpu.steps, 4);
    assert_eq!(machine.cpu_secondary.as_ref().unwrap().steps, 4);
    assert_eq!(machine.total_cycles, 4);
}

#[test]
fn cycle_limit_stops_at_atomic_batch_boundary() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new());
    machine.config.peripheral_tick_interval = 64;

    let report = machine
        .advance(AdvanceRequest::run(None).with_cycle_limit(3))
        .unwrap();

    assert_eq!(report.stop, AdvanceStop::CycleLimit);
    assert_eq!(report.elapsed_cycles, 3);
    assert_eq!(report.fuel_consumed, 3);
}

#[test]
fn zero_cycle_limit_does_not_mutate_machine() {
    let mut machine = counting_dual_core_machine();
    let before = serde_json::to_value(machine.snapshot()).unwrap();

    let report = machine
        .advance(AdvanceRequest::run(None).with_cycle_limit(0))
        .unwrap();

    assert_eq!(report.stop, AdvanceStop::CycleLimit);
    assert_eq!(report.fuel_consumed, 0);
    assert_eq!(report.elapsed_cycles, 0);
    assert_eq!(serde_json::to_value(machine.snapshot()).unwrap(), before);
    assert_eq!(machine.step_profile(), StepProfile::default());
}

#[test]
fn fuel_wins_when_fuel_and_cycle_limits_are_reached_together() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new());

    let report = machine
        .advance(AdvanceRequest::run(Some(3)).with_cycle_limit(3))
        .unwrap();

    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!((report.fuel_consumed, report.elapsed_cycles), (3, 3));
}

#[test]
fn breakpoint_wins_when_reached_exactly_at_fuel_limit() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new());
    machine.add_breakpoint(4);

    let report = machine.advance(AdvanceRequest::run(Some(2))).unwrap();

    assert_eq!(report.stop, AdvanceStop::Breakpoint(4));
    assert_eq!(report.fuel_consumed, 2);
    assert_eq!(report.primary_steps, 2);
    assert_eq!(report.elapsed_cycles, 2);
    assert_eq!(machine.last_breakpoint, Some(4));

    let mut adapter = Machine::new(CountingCpu::default(), SystemBus::new());
    adapter.add_breakpoint(4);
    assert_eq!(adapter.run(Some(2)).unwrap(), StopReason::Breakpoint(4));
}

#[test]
fn honor_breakpoint_stops_before_current_pc_and_inside_wide_window() {
    let mut current = Machine::new(CountingCpu::default(), SystemBus::new());
    current.add_breakpoint(0);
    let report = current.advance(AdvanceRequest::run(Some(8))).unwrap();
    assert_eq!(report.stop, AdvanceStop::Breakpoint(0));
    assert_eq!(report.primary_steps, 0);

    let mut inside = Machine::new(CountingCpu::default(), SystemBus::new());
    inside.config.peripheral_tick_interval = 64;
    inside.add_breakpoint(4);
    let report = inside.advance(AdvanceRequest::run(Some(8))).unwrap();
    assert_eq!(report.stop, AdvanceStop::Breakpoint(4));
    assert_eq!(report.primary_steps, 2);
    assert_eq!(inside.cpu.pc, 4);
}

#[test]
fn ignore_breakpoints_neither_stops_nor_touches_sticky_state() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new());
    machine.add_breakpoint(0);
    machine.last_breakpoint = Some(0);

    let report = machine
        .advance(AdvanceRequest::run(Some(2)).with_breakpoints(BreakpointPolicy::Ignore))
        .unwrap();

    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!(report.primary_steps, 2);
    assert_eq!(machine.last_breakpoint, Some(0));
}

#[test]
fn ignored_single_at_sticky_halted_pc_preserves_rearm_sequence() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new());
    machine.cpu.halt();
    machine.add_breakpoint(0);
    assert_eq!(
        machine.advance(AdvanceRequest::run(Some(1))).unwrap().stop,
        AdvanceStop::Breakpoint(0)
    );

    machine.advance(AdvanceRequest::single()).unwrap();
    assert_eq!(machine.last_breakpoint, Some(0));
    assert_eq!(
        machine.advance(AdvanceRequest::run(Some(1))).unwrap().stop,
        AdvanceStop::Breakpoint(0)
    );
}

#[test]
fn zero_progress_restores_clocks_and_does_not_profile_a_batch() {
    let mut machine = Machine::new(
        CountingCpu {
            zero_batch: true,
            ..Default::default()
        },
        SystemBus::new(),
    );
    machine.total_cycles = 9;
    machine.bus.set_current_cycle(9);

    let report = machine.advance(AdvanceRequest::run(Some(4))).unwrap();

    assert_eq!(report.stop, AdvanceStop::NoProgress);
    assert_eq!(report.primary_steps, 0);
    assert_eq!(report.cpu_batches, 0);
    assert_eq!(machine.total_cycles, 9);
    assert_eq!(machine.bus.current_cycle, 9);
    assert_eq!(machine.step_profile(), StepProfile::default());
}

#[test]
fn debug_reset_resets_both_cores() {
    let mut machine = counting_dual_core_machine();
    machine.cpu.pc = 0x1111;
    machine.cpu.sp = 0x2222;
    machine.cpu.steps = 3;
    machine.cpu.pending.push(1);
    machine.cpu.halted = true;
    let secondary = machine.cpu_secondary.as_mut().unwrap();
    secondary.pc = 0x3333;
    secondary.sp = 0x4444;
    secondary.steps = 5;
    secondary.pending.push(2);
    secondary.halted = true;

    DebugControl::reset(&mut machine).unwrap();

    assert_eq!(
        (machine.cpu.pc, machine.cpu.sp, machine.cpu.steps),
        (0, 0, 0)
    );
    assert!(machine.cpu.pending.is_empty());
    assert!(!machine.cpu.halted);
    let secondary = machine.cpu_secondary.as_ref().unwrap();
    assert_eq!((secondary.pc, secondary.sp, secondary.steps), (0, 0, 0));
    assert!(secondary.pending.is_empty());
    assert!(!secondary.halted);
}

#[test]
fn zero_tick_interval_matches_interval_one() {
    let mut zero = Machine::new(CountingCpu::default(), SystemBus::new());
    let mut one = Machine::new(CountingCpu::default(), SystemBus::new());
    zero.config.peripheral_tick_interval = 0;
    one.config.peripheral_tick_interval = 1;

    let zero_report = zero.advance(AdvanceRequest::run(Some(3))).unwrap();
    let one_report = one.advance(AdvanceRequest::run(Some(3))).unwrap();

    assert_eq!(zero_report, one_report);
    assert_eq!(zero.cpu.steps, one.cpu.steps);
    assert_eq!(zero.total_cycles, one.total_cycles);
    assert_eq!(zero.step_profile(), one.step_profile());
}

#[test]
fn primary_error_in_single_mode_preserves_precommit_state() {
    let mut machine = Machine::new(
        CountingCpu {
            fail_step: true,
            ..Default::default()
        },
        SystemBus::new(),
    );

    let error = machine.advance(AdvanceRequest::single()).unwrap_err();

    assert!(matches!(
        error,
        SimulationError::Other(ref message)
            if message == "CountingCpu injected step failure"
    ));
    assert_eq!(machine.total_cycles, 1);
    assert_eq!(machine.bus.current_cycle, 1);
    assert_eq!(machine.step_profile(), StepProfile::default());
}

#[test]
fn cpu_error_preserves_pending_scb_reset_and_aircr_fields() {
    const AIRCR: u64 = 0xE000_ED0C;
    const PRIGROUP: u32 = 5 << 8;
    let mut bus = SystemBus::new();
    let (_cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(
        CountingCpu {
            fail_step: true,
            ..Default::default()
        },
        bus,
    );
    machine
        .bus
        .write_u32(AIRCR, (0x05fa << 16) | PRIGROUP | (1 << 2))
        .unwrap();

    assert!(machine.advance(AdvanceRequest::run(Some(1))).is_err());

    assert_eq!(machine.bus.read_u32(AIRCR).unwrap() & 0x700, PRIGROUP);
    assert_ne!(machine.bus.read_u32(AIRCR).unwrap() & (1 << 2), 0);
}

#[test]
fn mid_batch_error_keeps_cpu_partial_progress_uncommitted() {
    let mut machine = Machine::new(
        CountingCpu {
            fail_batch_after: Some(2),
            ..Default::default()
        },
        SystemBus::new(),
    );
    machine.config.peripheral_tick_interval = 64;

    assert!(machine.advance(AdvanceRequest::run(Some(4))).is_err());
    assert_eq!(machine.cpu.steps, 2);
    assert_eq!(machine.total_cycles, 0);
    assert_eq!(machine.bus.current_cycle, 0);
    assert_eq!(machine.step_profile(), StepProfile::default());
}

fn rtc_reset_machine() -> Machine<CountingCpu> {
    use crate::peripherals::esp32::rtc_cntl::RtcCntl;

    let mut bus = SystemBus::new();
    bus.add_peripheral(
        "rtc_cntl",
        u64::from(RtcCntl::BASE),
        0x1000,
        None,
        Box::new(RtcCntl::new()),
    );
    let mut machine = Machine::new(CountingCpu::default(), bus);
    machine.config.peripheral_tick_interval = 64;
    machine
        .bus
        .write_u32(u64::from(RtcCntl::BASE), 1 << 31)
        .unwrap();
    machine
}

#[test]
fn rtc_reset_is_drained_at_the_first_unified_boundary() {
    let mut machine = rtc_reset_machine();

    let report = machine.advance(AdvanceRequest::run(Some(8))).unwrap();

    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!(report.fuel_consumed, 8);
    assert_eq!(report.primary_steps, 8);
    assert_eq!(report.secondary_steps, 0);
    assert_eq!(report.elapsed_cycles, 8);
    assert_eq!(report.idle_cycles, 0);
    assert_eq!(report.cpu_batches, 8);
    assert_eq!(machine.cpu.pc, 0x4000_0400 + 14);
    assert_eq!(machine.cpu.sp, 0x3FFE_0000);
}

#[test]
fn rtc_reset_is_drained_through_debug_run_adapter() {
    let mut machine = rtc_reset_machine();

    assert_eq!(machine.run(Some(8)).unwrap(), StopReason::MaxStepsReached);
    assert_eq!(machine.cpu.pc, 0x4000_0400 + 14);
    assert_eq!(machine.cpu.sp, 0x3FFE_0000);
}

#[test]
fn run_batch_cap_one_preserves_paused_push_last_write_wins() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new());
    machine.cpu.push_level = Some(false);
    arm_synthetic_push_channel(&mut machine);
    machine.bus.logic_tap.push(0, true);

    let report = machine
        .advance(
            AdvanceRequest::run(Some(1))
                .with_batch_cap(NonZeroU32::new(1).unwrap())
                .with_breakpoints(BreakpointPolicy::Ignore),
        )
        .unwrap();

    assert_eq!(report.cpu_batches, 1);
    assert!(machine.logic_read_edges(0).edges.is_empty());
}

#[test]
fn push_capture_does_not_clamp_scb_free_auto_batch() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new());
    machine.config.peripheral_tick_interval = 64;
    machine.cpu.push_level = Some(true);
    arm_synthetic_push_channel(&mut machine);

    let request = AdvanceRequest::run(Some(8));
    assert_eq!(request.batch_policy(), BatchPolicy::Auto);
    let report = machine.advance(request).unwrap();
    let profile = machine.step_profile();
    let edges = machine.logic_read_edges(0).edges;

    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!(report.primary_steps, 8);
    assert_eq!(report.cpu_batches, 1);
    assert_eq!(profile.cpu_instructions, 8);
    assert_eq!(profile.cpu_batches, 1);
    assert_eq!(machine.total_cycles, 8);
    assert_eq!(edges.len(), 1);
    assert_eq!((edges[0].ch, edges[0].cycle, edges[0].value), (0, 1, true));
    assert!(profile.cpu_batches < profile.cpu_instructions);
}

#[test]
fn run_dual_stamps_both_cores_at_one_end_boundary() {
    let mut machine = counting_dual_core_machine();
    machine.cpu.push_level = Some(true);
    machine.cpu_secondary.as_mut().unwrap().push_level = Some(false);
    arm_synthetic_push_channel(&mut machine);

    let report = machine.advance(AdvanceRequest::run(Some(1))).unwrap();

    assert_eq!((report.primary_steps, report.secondary_steps), (1, 1));
    assert!(
        machine.logic_read_edges(0).edges.is_empty(),
        "primary true and secondary false at the same end boundary collapse to the final level"
    );
}

#[derive(Debug)]
struct CostTicker;

impl crate::Peripheral for CostTicker {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        crate::PeripheralTickResult {
            cycles: 1,
            ..Default::default()
        }
    }
}

#[test]
fn cycle_limit_allows_atomic_boundary_tick_cost_overshoot() {
    let mut bus = SystemBus::new();
    bus.add_peripheral("cost", 0x5100_0000, 0x100, None, Box::new(CostTicker));
    let mut machine = Machine::new(CountingCpu::default(), bus);
    machine.config.peripheral_tick_interval = 1;

    let report = machine
        .advance(AdvanceRequest::run(None).with_cycle_limit(1))
        .unwrap();

    assert_eq!(report.stop, AdvanceStop::CycleLimit);
    assert_eq!(report.primary_steps, 1);
    assert_eq!(report.fuel_consumed, 1);
    assert_eq!(report.elapsed_cycles, 2);
    assert_eq!(machine.step_profile().peripheral_ticks, 1);
}

#[cfg(feature = "event-scheduler")]
#[test]
fn idle_fast_forward_is_reconciled_in_report_and_cpu() {
    let mut bus = SystemBus::new();
    bus.peripherals.clear();
    let mut machine = Machine::new(
        CountingCpu {
            idle_budget: Some(5),
            ..Default::default()
        },
        bus,
    );
    machine.config.idle_fast_forward_enabled = true;

    let report = machine.advance(AdvanceRequest::run(Some(5))).unwrap();

    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!(report.fuel_consumed, 5);
    assert_eq!(report.idle_cycles, 5);
    assert_eq!(report.primary_steps, 0);
    assert_eq!(report.cpu_batches, 0);
    assert_eq!(report.elapsed_cycles, 5);
    assert_eq!(machine.cpu.idle_skipped, 5);
    assert_eq!(machine.idle_fast_forward_cycles_skipped, 5);
}

#[cfg(feature = "event-scheduler")]
#[test]
fn ignored_breakpoint_does_not_block_idle_fast_forward() {
    let mut bus = SystemBus::new();
    bus.peripherals.clear();
    let mut machine = Machine::new(
        CountingCpu {
            idle_budget: Some(3),
            ..Default::default()
        },
        bus,
    );
    machine.config.idle_fast_forward_enabled = true;
    machine.add_breakpoint(0);

    let report = machine
        .advance(AdvanceRequest::run(Some(3)).with_breakpoints(BreakpointPolicy::Ignore))
        .unwrap();

    assert_eq!(report.idle_cycles, 3);
    assert_eq!(machine.cpu.steps, 0);
    assert_eq!(machine.last_breakpoint, None);
}

#[cfg(feature = "event-scheduler")]
#[test]
fn terminal_idle_skip_flushes_pending_push_observation() {
    let mut bus = SystemBus::new();
    bus.peripherals.clear();
    let mut machine = Machine::new(
        CountingCpu {
            idle_budget: Some(2),
            ..Default::default()
        },
        bus,
    );
    machine.config.idle_fast_forward_enabled = true;
    arm_synthetic_push_channel(&mut machine);
    machine.bus.logic_tap.push(0, true);

    let report = machine.advance(AdvanceRequest::run(Some(2))).unwrap();

    assert_eq!(report.idle_cycles, 2);
    assert_eq!(machine.logic_read_edges(0).edges.len(), 1);
}

#[test]
fn debug_run_adapter_matches_advance_single_core_contract() {
    let mut adapter = Machine::new(CountingCpu::default(), SystemBus::new());
    let mut direct = Machine::new(CountingCpu::default(), SystemBus::new());
    adapter.config.peripheral_tick_interval = 64;
    direct.config.peripheral_tick_interval = 64;

    let adapter_stop = adapter.run(Some(7)).unwrap();
    let report = direct.advance(AdvanceRequest::run(Some(7))).unwrap();

    assert_eq!(adapter_stop, StopReason::MaxStepsReached);
    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!(report.fuel_consumed, 7);
    assert_eq!(report.primary_steps, 7);
    assert_eq!(report.secondary_steps, 0);
    assert_eq!(report.elapsed_cycles, 7);
    assert_eq!(report.idle_cycles, 0);
    assert_eq!(report.cpu_batches, 1);
    assert_eq!(
        serde_json::to_value(direct.cpu.snapshot()).unwrap(),
        serde_json::to_value(adapter.cpu.snapshot()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(direct.snapshot()).unwrap(),
        serde_json::to_value(adapter.snapshot()).unwrap()
    );
    assert_eq!(direct.total_cycles, adapter.total_cycles);
    assert_eq!(direct.bus.current_cycle, adapter.bus.current_cycle);
    assert_eq!(direct.step_profile(), adapter.step_profile());
}

#[test]
fn step_adapter_matches_advance_single_contract() {
    let mut adapter = counting_dual_core_machine();
    let mut direct = counting_dual_core_machine();

    adapter.step().unwrap();
    let report = direct.advance(AdvanceRequest::single()).unwrap();

    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!((report.primary_steps, report.secondary_steps), (1, 1));
    assert_eq!(report.fuel_consumed, 1);
    assert_eq!(report.elapsed_cycles, 1);
    assert_eq!(report.idle_cycles, 0);
    assert_eq!(report.cpu_batches, 1);
    assert_eq!(
        serde_json::to_value(direct.cpu.snapshot()).unwrap(),
        serde_json::to_value(adapter.cpu.snapshot()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(direct.cpu_secondary.as_ref().unwrap().snapshot()).unwrap(),
        serde_json::to_value(adapter.cpu_secondary.as_ref().unwrap().snapshot()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(direct.snapshot()).unwrap(),
        serde_json::to_value(adapter.snapshot()).unwrap()
    );
    assert_eq!(direct.total_cycles, adapter.total_cycles);
    assert_eq!(direct.bus.current_cycle, adapter.bus.current_cycle);
    assert_eq!(direct.step_profile(), adapter.step_profile());
}

fn arm_synthetic_push_channel(machine: &mut Machine<CountingCpu>) {
    machine
        .logic_capture
        .install(&[Some((usize::MAX, 0))], &[Some(false)], &[true]);
    machine.bus.logic_tap.clear_events();
    machine.bus.logic_tap.set_clock(machine.total_cycles + 1);
    machine.bus.logic_tap.set_armed(true);
}

#[test]
fn advance_single_preserves_paused_push_same_boundary_last_write_wins() {
    let mut machine = Machine::new(CountingCpu::default(), SystemBus::new());
    machine.cpu.push_level = Some(false);
    arm_synthetic_push_channel(&mut machine);

    machine.bus.logic_tap.push(0, true);
    machine.advance(AdvanceRequest::single()).unwrap();

    let batch = machine.logic_read_edges(0);
    assert!(
        batch.edges.is_empty(),
        "paused true then instruction false at one boundary must be invisible"
    );
    assert_eq!(machine.total_cycles, 1);
    assert_eq!(machine.bus.current_cycle, 1);
    assert_eq!(machine.step_profile().cpu_instructions, 1);
}

#[test]
fn advance_single_preserves_primary_accounting_on_secondary_error() {
    let mut machine = counting_dual_core_machine();
    machine.cpu_secondary.as_mut().unwrap().fail_step = true;
    machine.config.peripheral_tick_interval = 1;

    let error = machine.advance(AdvanceRequest::single()).unwrap_err();

    assert!(matches!(
        error,
        SimulationError::Other(ref message)
            if message == "CountingCpu injected step failure"
    ));
    assert_eq!(machine.cpu.steps, 1);
    assert_eq!(machine.cpu_secondary.as_ref().unwrap().steps, 0);
    assert_eq!(machine.total_cycles, 1);
    assert_eq!(machine.bus.current_cycle, 1);
    assert_eq!(machine.step_profile().cpu_instructions, 1);
    assert_eq!(machine.step_profile().cpu_batches, 1);
    assert_eq!(machine.step_profile().peripheral_ticks, 0);
}

fn assert_real_cpu_step_adapter_boundary<C: Cpu>(mut machine: Machine<C>, expected_pc: u32) {
    machine.step().unwrap();

    assert_eq!(machine.cpu.get_pc(), expected_pc);
    assert_eq!(machine.total_cycles, 1);
    assert_eq!(machine.bus.current_cycle, 1);
    assert_eq!(machine.step_profile().cpu_instructions, 1);
    assert_eq!(machine.step_profile().cpu_batches, 1);
}

#[test]
fn arm_step_adapter_commits_one_boundary() {
    let mut bus = SystemBus::new();
    let (mut cpu, _) = crate::system::cortex_m::configure_cortex_m(&mut bus);
    bus.write_u16(0, 0xBF00).unwrap();
    cpu.set_pc(0);

    assert_real_cpu_step_adapter_boundary(Machine::new(cpu, bus), 2);
}

#[test]
fn riscv_step_adapter_commits_one_boundary() {
    let mut bus = SystemBus::new();
    let mut cpu = crate::system::riscv::configure_riscv(&mut bus);
    bus.write_u32(0, 0x0000_0013).unwrap();
    cpu.set_pc(0);

    assert_real_cpu_step_adapter_boundary(Machine::new(cpu, bus), 4);
}

#[test]
fn xtensa_step_adapter_commits_one_boundary() {
    let mut bus = SystemBus::new();
    let mut cpu = crate::cpu::xtensa_lx7::XtensaLx7::new();
    bus.write_u8(0, 0x3d).unwrap();
    bus.write_u8(1, 0xf0).unwrap();
    cpu.set_pc(0);

    assert_real_cpu_step_adapter_boundary(Machine::new(cpu, bus), 2);
}

struct AppCpuBootAddrReset;

impl Drop for AppCpuBootAddrReset {
    fn drop(&mut self) {
        crate::peripherals::esp_xtensa_common::rom_thunks::APPCPU_BOOT_ADDR
            .with(|slot| slot.set(None));
    }
}

#[test]
fn unified_single_releases_and_steps_app_cpu() {
    let _reset = AppCpuBootAddrReset;
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
