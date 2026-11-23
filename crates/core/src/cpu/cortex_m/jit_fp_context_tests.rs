// SPDX-License-Identifier: MIT
//! Integer blocks remain usable while FP blocks preserve ExecuteFPCheck.

use super::*;
use crate::cpu::jit_framework::cortex_m::{CortexMJitEngine, EngineStats};
use crate::peripherals::cortex_m_fpu::{FpContextState, FPDSCR_MASK};
use crate::peripherals::scb::{
    ScbFaultState, CFSR_UFSR_NOCP, CPACR_CP10_CP11_MASK, SHCSR_USGFAULTENA,
};

const INTEGER_PC: u32 = 0x100;
const FP_PC: u32 = 0x120;
const SAVE_ADDR: u32 = 0x2000_1000;

struct Fixture {
    cpu: CortexM,
    bus: SystemBus,
    fp: Arc<FpContextState>,
    cpacr: Arc<AtomicU32>,
    faults: Arc<ScbFaultState>,
}

impl Fixture {
    fn new() -> Self {
        let mut cpu = CortexM::new();
        let mut bus = SystemBus::new();
        let fp = Arc::new(FpContextState::default());
        let cpacr = Arc::new(AtomicU32::new(CPACR_CP10_CP11_MASK));
        let faults = Arc::new(ScbFaultState::new(true));
        faults.shcsr.store(SHCSR_USGFAULTENA, Ordering::Relaxed);
        cpu.set_shared_fp_context(fp.clone());
        cpu.set_shared_cpacr(cpacr.clone());
        cpu.set_shared_faults(faults.clone());
        cpu.pc = INTEGER_PC;
        cpu.sp = 0x2000_8000;
        cpu.xpsr = 1 << 24;
        assert!(bus.flash.write_u32(6 * 4, 0x201));
        assert!(bus.flash.write_u16(0x200, 0xe7fe));
        Self {
            cpu,
            bus,
            fp,
            cpacr,
            faults,
        }
    }

    fn code(&mut self, pc: u32, halfwords: &[u16]) {
        for (i, halfword) in halfwords.iter().enumerate() {
            assert!(self
                .bus
                .flash
                .write_u16(u64::from(pc) + 2 * i as u64, *halfword));
        }
    }

    fn batch(&mut self, count: u32) -> u32 {
        let config = SimulationConfig {
            cortex_m_jit_enabled: true,
            cortex_m_jit_min_block_instrs: 2,
            ..SimulationConfig::default()
        };
        self.cpu
            .step_batch(&mut self.bus, &[], &config, count)
            .unwrap()
    }

    fn stats(&self) -> EngineStats {
        self.cpu.jit_stats().expect("native JIT entered")
    }

    fn warm_chained_blocks(&mut self) {
        self.code(INTEGER_PC, &[0x3001, branch(INTEGER_PC + 2, FP_PC)]);
        // VMOV.F32 s0, s1; branch to the integer block.
        self.code(FP_PC, &[0xeeb0, 0x0a60, branch(FP_PC + 4, INTEGER_PC)]);
        self.cpu.control = 4;
        self.cpu.fpu_s[1] = 1.25f32.to_bits();
        let mut engine = CortexMJitEngine::new(50);
        engine.set_min_profitable(2);
        engine.try_compile_from_bus(u64::from(INTEGER_PC), &self.bus);
        engine.try_compile_from_bus(u64::from(FP_PC), &self.bus);
        assert_eq!(engine.stats().compiled, 2);
        self.cpu.jit_engine = Some(engine);
        assert_eq!(self.batch(4), 4);
        assert_eq!(self.stats().block_runs, 2);
        assert_eq!(self.stats().chained, 1);
        assert_eq!(self.cpu.pc, INTEGER_PC);
    }
}

fn branch(from: u32, to: u32) -> u16 {
    0xe000 | (((i64::from(to) - i64::from(from) - 4) / 2) as u16 & 0x7ff)
}

#[test]
fn integer_blocks_compile_with_denied_uninitialized_or_lazy_fp_context() {
    // Both the register-only and host-memory bindings must stay usable.
    for first in [0x3001, 0x6808] {
        // adds r0, #1; or ldr r0, [r1]
        for (access, control, lazy) in [
            (0, 0, false),
            (CPACR_CP10_CP11_MASK, 0, false),
            (CPACR_CP10_CP11_MASK, 4, true),
        ] {
            let mut f = Fixture::new();
            f.code(INTEGER_PC, &[first, branch(INTEGER_PC + 2, INTEGER_PC)]);
            f.cpacr.store(access, Ordering::Relaxed);
            f.cpu.control = control;
            f.fp.set_fpcar(SAVE_ADDR);
            f.fp.set_lspact(lazy);
            f.cpu.r1 = 0x2000_0000;
            f.bus.write_u32(0x2000_0000, 42).unwrap();
            f.bus.ram.data[0x1000..0x1044].fill(0xa5);
            f.cpu.fpu_s.fill(0x3f80_0000);
            f.cpu.fpscr = 0x8100_0000;
            let fpccr = f.fp.fpccr();
            assert_eq!(f.batch(200), 200);
            assert!(
                f.stats().compiled > 0 && f.stats().block_runs > 0,
                "{access:#x}/{control}/{lazy}: {:?}",
                f.stats()
            );
            assert_eq!(f.cpu.r0, if first == 0x3001 { 100 } else { 42 });
            assert_eq!(f.cpu.control, control);
            assert_eq!(f.fp.fpccr(), fpccr);
            assert_eq!(f.cpu.fpu_s, [0x3f80_0000; 32]);
            assert_eq!(f.cpu.fpscr, 0x8100_0000);
            assert!(f.bus.ram.data[0x1000..0x1044]
                .iter()
                .all(|byte| *byte == 0xa5));
        }
    }
}

#[test]
fn warmed_direct_fp_block_rechecks_access_and_privilege() {
    for (access, control) in [(0, 4), (0b11 << 20, 4), ((0b01 << 20) | (0b01 << 22), 5)] {
        let mut f = Fixture::new();
        f.warm_chained_blocks();
        let mut engine = f.cpu.jit_engine.take().unwrap();
        let before = engine.stats();
        f.cpacr.store(access, Ordering::Relaxed);
        f.cpu.control = control;
        f.cpu.pc = FP_PC;
        f.cpu.fpu_s[0] = 7.0f32.to_bits();
        f.cpu.fpu_s[1] = 9.0f32.to_bits();
        let mut machine = crate::Machine::new(f.cpu, f.bus);
        assert_eq!(engine.step_unit(&mut machine), 1);
        assert_eq!(machine.cpu.fpu_s[0], 7.0f32.to_bits());
        assert_eq!(machine.cpu.pc, FP_PC);
        assert_ne!(f.faults.cfsr.load(Ordering::Relaxed) & CFSR_UFSR_NOCP, 0);
        assert_eq!(engine.stats().block_runs, before.block_runs);
        assert_eq!(engine.stats().compiled, before.compiled);
        machine.step().unwrap();
        assert_eq!(machine.cpu.active_exception, 6);
        assert_eq!(machine.cpu.pc, 0x200);
    }
}

#[test]
fn warmed_direct_fp_block_initializes_a_new_context_before_running() {
    let mut f = Fixture::new();
    f.warm_chained_blocks();
    let mut engine = f.cpu.jit_engine.take().unwrap();
    let before = engine.stats();
    f.cpu.pc = FP_PC;
    f.cpu.control = 0;
    f.cpu.fpscr = 0x8000_0000 | FPDSCR_MASK;
    f.cpu.fpu_s[1] = 3.5f32.to_bits();
    let mut machine = crate::Machine::new(f.cpu, f.bus);
    assert_eq!(engine.step_unit(&mut machine), 1);
    assert_eq!(machine.cpu.control & 4, 4);
    assert_eq!(machine.cpu.fpscr, 0x8000_0000);
    assert_eq!(machine.cpu.fpu_s[0], 3.5f32.to_bits());
    assert_eq!(engine.stats().block_runs, before.block_runs);
    machine.cpu.pc = FP_PC;
    assert_eq!(engine.step_unit(&mut machine), 2);
    assert_eq!(engine.stats().compiled, before.compiled);
    assert_eq!(engine.stats().block_runs, before.block_runs + 1);
}

#[test]
fn chained_fp_block_rechecks_revoked_permission_after_integer_execution() {
    let mut f = Fixture::new();
    f.warm_chained_blocks();
    let before = f.stats();
    f.cpacr.store(0, Ordering::Relaxed);
    f.cpu.fpu_s[0] = 7.0f32.to_bits();
    f.cpu.fpu_s[1] = 9.0f32.to_bits();
    assert_eq!(f.batch(4), 3);
    assert_eq!(f.cpu.r0, 2);
    assert_eq!(f.cpu.fpu_s[0], 7.0f32.to_bits());
    assert_ne!(f.faults.cfsr.load(Ordering::Relaxed) & CFSR_UFSR_NOCP, 0);
    assert_eq!(f.stats().compiled, before.compiled);
    assert_eq!(f.stats().block_runs, before.block_runs + 1);
    assert_eq!(f.stats().chained, before.chained);
}

#[test]
fn chained_fp_block_preserves_lazy_state_before_its_first_fp_write() {
    let mut f = Fixture::new();
    f.warm_chained_blocks();
    let before = f.stats();
    for (i, value) in f.cpu.fpu_s.iter_mut().enumerate() {
        *value = 0x4000_0000 + i as u32;
    }
    let saved = f.cpu.fpu_s;
    f.cpu.fpscr = 0x2100_0000;
    f.bus.ram.data[0x1000..0x1044].fill(0xa5);
    // Enter a real lazy-stacked handler at the already compiled integer block.
    f.cpu.set_sp(SAVE_ADDR + 0x48);
    assert!(f.bus.flash.write_u32(16 * 4, INTEGER_PC | 1));
    f.cpu.set_exception_pending(16);
    f.cpu
        .step(&mut f.bus, &[], &SimulationConfig::default())
        .unwrap();
    assert_eq!(f.cpu.active_exception, 16);
    assert_eq!(f.cpu.control & 4, 0);
    assert_eq!(f.fp.fpcar(), SAVE_ADDR);
    assert!(f.fp.lspact());
    assert!(f.bus.ram.data[0x1000..0x1044]
        .iter()
        .all(|byte| *byte == 0xa5));
    assert_eq!(f.batch(4), 4);
    assert_eq!(f.stats().compiled, before.compiled);
    assert_eq!(f.stats().block_runs, before.block_runs + 1);
    assert_eq!(f.stats().chained, before.chained);
    assert!(!f.fp.lspact());
    for (i, value) in saved.iter().take(16).enumerate() {
        assert_eq!(
            f.bus.read_u32(u64::from(SAVE_ADDR) + 4 * i as u64).unwrap(),
            *value
        );
    }
    assert_eq!(
        f.bus.read_u32(u64::from(SAVE_ADDR) + 0x40).unwrap(),
        0x2100_0000
    );
    assert_eq!(f.cpu.fpu_s[0], saved[1]);
    assert_eq!(f.cpu.pc, INTEGER_PC);
    assert_eq!(f.batch(4), 4);
    assert_eq!(f.stats().block_runs, before.block_runs + 3);
    assert_eq!(f.stats().chained, before.chained + 1);
}

#[test]
fn warmed_mixed_block_retires_integer_prefix_once_before_fp_fault() {
    let mut f = Fixture::new();
    // adds r0, #1; VMOV.F32 s0, s1; branch to the mixed block.
    f.code(
        INTEGER_PC,
        &[0x3001, 0xeeb0, 0x0a60, branch(INTEGER_PC + 6, INTEGER_PC)],
    );
    f.cpu.control = 4;
    f.cpu.fpu_s[1] = 1.25f32.to_bits();
    let mut engine = CortexMJitEngine::new(50);
    engine.set_min_profitable(2);
    engine.try_compile_from_bus(u64::from(INTEGER_PC), &f.bus);
    assert_eq!(engine.stats().compiled, 1);
    f.cpu.jit_engine = Some(engine);
    assert_eq!(f.batch(3), 3);
    assert_eq!(f.cpu.r0, 1);
    let before = f.stats();
    assert_eq!(before.block_runs, 1);
    f.cpacr.store(0, Ordering::Relaxed);
    f.cpu.fpu_s[0] = 7.0f32.to_bits();
    f.cpu.fpu_s[1] = 9.0f32.to_bits();
    assert_eq!(f.batch(3), 2);
    assert_eq!(f.cpu.r0, 2, "the integer prefix retires exactly once");
    assert_eq!(
        f.cpu.pc,
        INTEGER_PC + 2,
        "fault identifies the FP instruction"
    );
    assert_eq!(f.cpu.fpu_s[0], 7.0f32.to_bits());
    assert_ne!(f.faults.cfsr.load(Ordering::Relaxed) & CFSR_UFSR_NOCP, 0);
    assert_eq!(f.stats().block_runs, before.block_runs);
    assert_eq!(f.stats().compiled, before.compiled);
    assert_eq!(f.batch(1), 1);
    assert_eq!(f.cpu.active_exception, 6);
    assert_eq!(
        f.bus.read_u32(u64::from(f.cpu.sp) + 24).unwrap(),
        INTEGER_PC + 2
    );
}
