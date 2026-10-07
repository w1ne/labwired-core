// SPDX-License-Identifier: MIT
//! Exact differential countdown retirement, not a cycle-accuracy claim.
use super::*;

fn state(cpu: &CortexM) -> serde_json::Value {
    serde_json::to_value(cpu.snapshot()).unwrap()
}

fn fixture(rd: u8, value: u32, flags: u32) -> (CortexM, SystemBus) {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    cpu.pc = 0x100;
    cpu.xpsr = flags | 1 << 24;
    cpu.write_reg(rd, value);
    for (pc, op) in [(0x100, 0x3801 | u16::from(rd) << 8), (0x102, 0xd1fd)] {
        assert!(bus.flash.write_u16(u64::from(pc), op));
        cpu.insert_decoded_entry(DecodeCacheEntry {
            tag: pc,
            instruction: decode_thumb_16(op),
            opcode: u32::from(op),
            pc_increment: 2,
            cycles: 1,
        });
    }
    (cpu, bus)
}

#[test]
fn countdown_matches_real_decoder_and_interpreter_for_registers_flags_and_budgets() {
    for rd in 0..8 {
        for value in [0, 1, 2, 3, 2048, u32::MAX, 0x80000000, 0x80000001] {
            for flags in [0, 0xf0000000, 0xa80f0000] {
                for budget in (0..=33).chain([127, 4095, 4096, 5000]) {
                    let (mut fast, fast_bus) = fixture(rd, value, flags);
                    let (mut ordinary, mut ordinary_bus) = fixture(rd, value, flags);
                    let before_counts = fast_bus.access_counts();
                    let retired = fast.run_t16_countdown(budget);
                    assert!(retired <= budget);
                    let config = ordinary_bus.config.clone();
                    for _ in 0..retired {
                        ordinary
                            .step_internal(&mut ordinary_bus, &[], &config)
                            .unwrap();
                    }
                    assert_eq!(
                        state(&fast),
                        state(&ordinary),
                        "rd={rd} value={value:#x} flags={flags:#x} budget={budget}"
                    );
                    assert_eq!(fast_bus.access_counts(), before_counts);
                    assert_eq!(ordinary_bus.access_counts(), before_counts);
                    if retired < budget {
                        assert_eq!((fast.pc, fast.read_reg(rd)), (0x104, 0));
                    }
                }
            }
        }
    }
}

#[test]
fn zero_wrap_signed_overflow_carry_and_odd_exit_keep_last_subs_flags() {
    for (initial, budget, expected, pc, nzcv) in [
        (0, 1, u32::MAX, 0x102, 0x80000000),
        (0x80000000, 2, 0x7fffffff, 0x100, 0x30000000),
        (1, 1, 0, 0x102, 0x60000000),
        (1, 2, 0, 0x104, 0x60000000),
        (2, 3, 0, 0x102, 0x60000000),
        (2, 17, 0, 0x104, 0x60000000),
    ] {
        let (mut cpu, _) = fixture(7, initial, 0xf0000000);
        let retired = cpu.run_t16_countdown(budget);
        assert_eq!(
            retired,
            budget.min(if initial == 0 {
                budget
            } else {
                initial.saturating_mul(2)
            })
        );
        assert_eq!(
            (cpu.r7, cpu.pc, cpu.xpsr & 0xf0000000),
            (expected, pc, nzcv)
        );
    }
}

#[test]
fn maximum_budget_and_initial_zero_do_not_overflow_retirement_math() {
    let (mut cpu, bus) = fixture(3, 0, 0);
    assert_eq!(cpu.run_t16_countdown(u32::MAX), u32::MAX);
    assert_eq!(cpu.r3, 0x80000000);
    assert_eq!(cpu.pc, 0x102);
    assert_eq!(cpu.xpsr & 0xf0000000, 0xa0000000);
    assert_eq!(bus.access_counts(), (0, 0, 0));
}

#[test]
fn branch_rotated_cold_wrong_opcode_tag_and_width_entries_are_side_effect_free() {
    for case in 0..9 {
        let (mut cpu, bus) = fixture(2, 20, 0xf0000000);
        match case {
            0 => cpu.pc += 2,
            1 => cpu.invalidate_code_caches(),
            2 => cpu.decode_cache[0x80].as_mut().unwrap().tag = 0x2100,
            3 => cpu.decode_cache[0x81].as_mut().unwrap().tag = 0x2102,
            4 => cpu.decode_cache[0x80].as_mut().unwrap().pc_increment = 4,
            5 => cpu.decode_cache[0x81].as_mut().unwrap().pc_increment = 4,
            6 => cpu.decode_cache[0x80].as_mut().unwrap().opcode = 0x3a02,
            7 => cpu.decode_cache[0x81].as_mut().unwrap().opcode = 0xd0fd,
            _ => cpu.decode_cache[0x81].as_mut().unwrap().opcode = 0xd1fc,
        }
        let before = (state(&cpu), bus.access_counts());
        assert_eq!(cpu.run_t16_countdown(33), 0);
        assert_eq!((state(&cpu), bus.access_counts()), before);
    }
}

#[test]
fn code_patch_invalidation_cannot_reuse_a_cached_countdown() {
    let (mut cpu, mut bus) = fixture(0, 20, 0);
    assert_eq!(cpu.run_t16_countdown(2), 2);
    assert!(bus.flash.write_u16(0x100, 0x3802));
    cpu.invalidate_code_caches();
    assert_eq!(cpu.run_t16_countdown(10), 0);
    let config = bus.config.clone();
    cpu.step_internal(&mut bus, &[], &config).unwrap();
    assert_eq!((cpu.pc, cpu.r0), (0x102, 17));
}

#[derive(Debug, Default)]
struct StepCounter(AtomicU32);

impl SimulationObserver for StepCounter {
    fn on_step_start(&self, _pc: u32, _opcode: u32) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn observers_receive_every_instruction_and_batched_clock_matches_ordinary_steps() {
    for budget in [8, 17, 33, 127] {
        let (mut observed, mut observed_bus) = fixture(4, 2048, 0);
        let (mut ordinary, mut ordinary_bus) = fixture(4, 2048, 0);
        let observer = Arc::new(StepCounter::default());
        let observers: Vec<Arc<dyn SimulationObserver>> = vec![observer.clone()];
        let mut config = observed_bus.config.clone();
        config.peripheral_tick_interval = 17;
        assert_eq!(
            observed
                .step_batch(&mut observed_bus, &observers, &config, budget)
                .unwrap(),
            budget
        );
        assert_eq!(observer.0.load(Ordering::Relaxed), budget);
        config.batch_mode_enabled = false;
        assert_eq!(
            ordinary
                .step_batch(&mut ordinary_bus, &[], &config, budget)
                .unwrap(),
            budget
        );
        assert_eq!(state(&observed), state(&ordinary));
        assert_eq!(observed_bus.current_cycle, ordinary_bus.current_cycle);
    }
}

#[test]
fn machine_scheduler_budgets_preserve_retirement_and_cycle_accounting() {
    use crate::{DebugControl, Machine};
    for interval in [1, 8, 17, 64] {
        let (cpu, bus) = fixture(5, 2048, 0);
        let (reference_cpu, reference_bus) = fixture(5, 2048, 0);
        let mut machine = Machine::new(cpu, bus);
        let mut reference = Machine::new(reference_cpu, reference_bus);
        machine.config.peripheral_tick_interval = interval;
        reference.config.peripheral_tick_interval = interval;
        reference.config.batch_mode_enabled = false;
        machine.run(Some(127)).unwrap();
        reference.run(Some(127)).unwrap();
        assert_eq!(state(&machine.cpu), state(&reference.cpu));
        assert_eq!(machine.total_cycles, reference.total_cycles);
        assert_eq!(machine.total_cycles, 127);
        assert_eq!(machine.step_profile().cpu_instructions, 127);
        assert_eq!(machine.bus.current_cycle, reference.bus.current_cycle);
    }
}

#[test]
fn pending_interrupt_takes_priority_over_the_countdown_fast_path() {
    let (mut cpu, mut bus) = fixture(6, 2048, 0);
    let (mut ordinary, mut ordinary_bus) = fixture(6, 2048, 0);
    for (core, memory) in [(&mut cpu, &mut bus), (&mut ordinary, &mut ordinary_bus)] {
        core.sp = 0x20001000;
        assert!(memory.flash.write_u32(15 * 4, 0x181));
        for pc in (0x180..0x1a0).step_by(2) {
            assert!(memory.flash.write_u16(pc, 0xbf00));
        }
        core.set_exception_pending(15);
    }
    let mut config = bus.config.clone();
    config.peripheral_tick_interval = 17;
    assert_eq!(cpu.step_batch(&mut bus, &[], &config, 9).unwrap(), 9);
    config.batch_mode_enabled = false;
    assert_eq!(
        ordinary
            .step_batch(&mut ordinary_bus, &[], &config, 9)
            .unwrap(),
        9
    );
    assert_eq!(state(&cpu), state(&ordinary));
    assert_eq!(cpu.r6, 2048, "IRQ handler runs before any SUBS retires");
    assert_eq!(cpu.active_exception, 15);
    assert_eq!(bus.access_counts(), ordinary_bus.access_counts());
    assert_eq!(bus.current_cycle, ordinary_bus.current_cycle);
}

#[test]
fn countdown_pc_wrap_matches_thumb_branch_target_and_odd_retirement() {
    for (initial, budget, pc) in [(1, 2, 2), (2, 3, 0)] {
        let (mut cpu, _) = fixture(0, initial, 0);
        cpu.pc = u32::MAX - 1;
        for (tag, opcode) in [(cpu.pc, 0x3801), (0, 0xd1fd)] {
            cpu.insert_decoded_entry(DecodeCacheEntry {
                tag,
                instruction: decode_thumb_16(opcode),
                opcode: u32::from(opcode),
                pc_increment: 2,
                cycles: 1,
            });
        }
        assert_eq!(cpu.run_t16_countdown(budget), budget);
        assert_eq!((cpu.r0, cpu.pc), (0, pc));
        assert_eq!(cpu.xpsr & 0xf0000000, 0x60000000);
    }
}
