use crate::bus::SystemBus;
use crate::runtime_snapshot::CpuKind;
use crate::snapshot::{ArmCpuSnapshot, CpuSnapshot};
use crate::{
    AdvanceRequest, AdvanceStop, BatchPolicy, BreakpointPolicy, Bus, Cpu, DebugControl, IdlePolicy,
    Machine, SimResult, SimulationConfig, SimulationError, SimulationObserver, StepProfile,
    StopReason,
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
}

fn counting_dual_core_machine() -> Machine<CountingCpu> {
    Machine::new(CountingCpu::default(), crate::bus::SystemBus::new())
        .with_secondary_cpu(CountingCpu::default())
}

#[test]
fn legacy_step_advances_both_cores_once() {
    let mut machine = counting_dual_core_machine();

    machine.step().expect("legacy step should succeed");

    assert_eq!(machine.cpu.steps, 1);
    assert_eq!(machine.cpu_secondary.as_ref().map(|cpu| cpu.steps), Some(1));
    assert_eq!(machine.total_cycles, 1);
}

#[test]
fn legacy_run_currently_omits_secondary_core() {
    let mut machine = counting_dual_core_machine();

    let reason = machine.run(Some(4)).expect("legacy run should succeed");

    assert_eq!(reason, StopReason::MaxStepsReached);
    assert_eq!(machine.cpu.steps, 4);
    assert_eq!(machine.cpu_secondary.as_ref().map(|cpu| cpu.steps), Some(0));
}

#[test]
fn legacy_single_step_publishes_and_profiles_one_cycle() {
    let mut machine = Machine::new(CountingCpu::default(), crate::bus::SystemBus::new());
    machine.reset_step_profile();

    machine.step().expect("legacy step should succeed");

    assert_eq!(machine.total_cycles, 1);
    assert_eq!(machine.bus.current_cycle, 1);
    let profile = machine.step_profile();
    assert_eq!(profile.cpu_instructions, 1);
    assert_eq!(profile.cpu_batches, 1);
}

#[test]
fn reset_step_profile_clears_dirty_counters() {
    let mut machine = Machine::new(CountingCpu::default(), crate::bus::SystemBus::new());
    machine.step().expect("legacy step should succeed");
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
fn legacy_dual_core_halted_primary_still_consumes_one_scheduling_quantum() {
    let mut machine = counting_dual_core_machine();
    machine.cpu.halt();

    machine.step().expect("legacy step should succeed");

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
fn advance_rejects_every_request_outside_exact_single_contract_without_mutation() {
    let unsupported = [
        ("run", AdvanceRequest::run(Some(1))),
        ("cycle limit", AdvanceRequest::single().with_cycle_limit(0)),
        (
            "breakpoint override",
            AdvanceRequest::single().with_breakpoints(BreakpointPolicy::Honor),
        ),
        (
            "batch cap",
            AdvanceRequest::single().with_batch_cap(NonZeroU32::new(2).unwrap()),
        ),
    ];

    for (case, request) in unsupported {
        let mut machine = counting_dual_core_machine();
        let machine_before = serde_json::to_value(machine.snapshot()).unwrap();
        let primary_before = serde_json::to_value(machine.cpu.snapshot()).unwrap();
        let secondary_before =
            serde_json::to_value(machine.cpu_secondary.as_ref().unwrap().snapshot()).unwrap();
        let cycles_before = machine.total_cycles;
        let bus_cycle_before = machine.bus.current_cycle;
        let profile_before = machine.step_profile();

        let error = machine.advance(request).unwrap_err();

        assert!(
            matches!(error, SimulationError::NotImplemented(_)),
            "{case}: unsupported requests must return an explicit error"
        );
        assert_eq!(
            serde_json::to_value(machine.snapshot()).unwrap(),
            machine_before,
            "{case}: machine snapshot changed"
        );
        assert_eq!(
            serde_json::to_value(machine.cpu.snapshot()).unwrap(),
            primary_before,
            "{case}: primary CPU changed"
        );
        assert_eq!(
            serde_json::to_value(machine.cpu_secondary.as_ref().unwrap().snapshot()).unwrap(),
            secondary_before,
            "{case}: secondary CPU changed"
        );
        assert_eq!(
            machine.total_cycles, cycles_before,
            "{case}: cycles changed"
        );
        assert_eq!(
            machine.bus.current_cycle, bus_cycle_before,
            "{case}: bus cycle changed"
        );
        assert_eq!(
            machine.step_profile(),
            profile_before,
            "{case}: profile changed"
        );
    }
}

#[test]
fn advance_single_is_byte_identical_to_legacy_step() {
    let mut legacy = counting_dual_core_machine();
    let mut unified = counting_dual_core_machine();

    legacy.step_legacy_for_test().unwrap();
    let report = unified.advance(AdvanceRequest::single()).unwrap();

    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!((report.primary_steps, report.secondary_steps), (1, 1));
    assert_eq!(report.fuel_consumed, 1);
    assert_eq!(report.elapsed_cycles, 1);
    assert_eq!(report.idle_cycles, 0);
    assert_eq!(report.cpu_batches, 1);
    assert_eq!(
        serde_json::to_value(unified.cpu.snapshot()).unwrap(),
        serde_json::to_value(legacy.cpu.snapshot()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(unified.cpu_secondary.as_ref().unwrap().snapshot()).unwrap(),
        serde_json::to_value(legacy.cpu_secondary.as_ref().unwrap().snapshot()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(unified.snapshot()).unwrap(),
        serde_json::to_value(legacy.snapshot()).unwrap()
    );
    assert_eq!(unified.total_cycles, legacy.total_cycles);
    assert_eq!(unified.bus.current_cycle, legacy.bus.current_cycle);
    assert_eq!(unified.step_profile(), legacy.step_profile());
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
    let mut legacy = Machine::new(CountingCpu::default(), SystemBus::new());
    let mut unified = Machine::new(CountingCpu::default(), SystemBus::new());
    legacy.cpu.push_level = Some(false);
    unified.cpu.push_level = Some(false);
    arm_synthetic_push_channel(&mut legacy);
    arm_synthetic_push_channel(&mut unified);

    legacy.bus.logic_tap.push(0, true);
    unified.bus.logic_tap.push(0, true);
    legacy.step_legacy_for_test().unwrap();
    unified.advance(AdvanceRequest::single()).unwrap();

    let legacy_batch = legacy.logic_read_edges(0);
    let unified_batch = unified.logic_read_edges(0);
    assert_eq!(unified_batch.cursor, legacy_batch.cursor);
    assert_eq!(unified_batch.dropped, legacy_batch.dropped);
    assert_eq!(unified_batch.edges, legacy_batch.edges);
    assert!(
        unified_batch.edges.is_empty(),
        "paused true then instruction false at one boundary must be invisible"
    );
    assert_eq!(unified.total_cycles, legacy.total_cycles);
    assert_eq!(unified.bus.current_cycle, legacy.bus.current_cycle);
    assert_eq!(unified.step_profile(), legacy.step_profile());
}

#[test]
fn advance_single_preserves_primary_accounting_on_secondary_error() {
    let mut legacy = counting_dual_core_machine();
    let mut unified = counting_dual_core_machine();
    legacy.cpu_secondary.as_mut().unwrap().fail_step = true;
    unified.cpu_secondary.as_mut().unwrap().fail_step = true;
    legacy.config.peripheral_tick_interval = 1;
    unified.config.peripheral_tick_interval = 1;

    let legacy_error = legacy.step_legacy_for_test().unwrap_err();
    let unified_error = unified.advance(AdvanceRequest::single()).unwrap_err();

    assert_eq!(unified_error.to_string(), legacy_error.to_string());
    assert_eq!(
        serde_json::to_value(unified.cpu.snapshot()).unwrap(),
        serde_json::to_value(legacy.cpu.snapshot()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(unified.cpu_secondary.as_ref().unwrap().snapshot()).unwrap(),
        serde_json::to_value(legacy.cpu_secondary.as_ref().unwrap().snapshot()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(unified.snapshot()).unwrap(),
        serde_json::to_value(legacy.snapshot()).unwrap()
    );
    assert_eq!(unified.total_cycles, legacy.total_cycles);
    assert_eq!(unified.bus.current_cycle, legacy.bus.current_cycle);
    assert_eq!(unified.step_profile(), legacy.step_profile());
    assert_eq!(unified.step_profile().cpu_instructions, 1);
    assert_eq!(unified.step_profile().cpu_batches, 1);
    assert_eq!(unified.step_profile().peripheral_ticks, 0);
}

fn assert_real_cpu_single_step_matches<C: Cpu>(factory: impl Fn() -> Machine<C>) {
    let mut legacy = factory();
    let mut unified = factory();

    legacy.step_legacy_for_test().unwrap();
    let report = unified.advance(AdvanceRequest::single()).unwrap();

    assert_eq!(report.stop, AdvanceStop::FuelLimit);
    assert_eq!(report.primary_steps, 1);
    assert_eq!(
        serde_json::to_value(legacy.cpu.snapshot()).unwrap(),
        serde_json::to_value(unified.cpu.snapshot()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(legacy.snapshot()).unwrap(),
        serde_json::to_value(unified.snapshot()).unwrap()
    );
    assert_eq!(legacy.total_cycles, unified.total_cycles);
    assert_eq!(legacy.bus.current_cycle, unified.bus.current_cycle);
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
