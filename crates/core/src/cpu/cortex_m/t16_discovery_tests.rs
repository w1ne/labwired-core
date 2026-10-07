// SPDX-License-Identifier: MIT
//! Admission/discovery regressions for mixed-width and unsupported hot loops.
use super::*;

#[test]
fn cached_scalar_matches_interpreter_for_every_halfword_and_flags() {
    // Reuse buses/caches to avoid making allocation throughput the test.
    let mut actual = CortexM::new();
    let mut reference = CortexM::new();
    let mut actual_bus = SystemBus::new();
    let mut reference_bus = SystemBus::new();
    let config = reference_bus.config.clone();
    for flags in [0x01000000, 0xa1000000, 0x71000000] {
        for opcode in 0..=u16::MAX {
            for cpu in [&mut actual, &mut reference] {
                cpu.pc = 0x100;
                cpu.xpsr = flags;
                cpu.r0 = 0x80000001;
                cpu.r1 = 0x20000100;
                cpu.r2 = 3;
                cpu.r3 = 0;
                cpu.r4 = 0xffffffff;
                cpu.r5 = 31;
                cpu.r6 = 0x7fffffff;
                cpu.r7 = 32;
                cpu.r8 = 255;
                cpu.r9 = 0;
                cpu.r10 = 1;
                cpu.r11 = 33;
                cpu.r12 = 0x12345678;
                cpu.sp = 0x20000200;
                cpu.lr = 0x105;
                cache(cpu, 0x100, opcode);
            }
            for bus in [&mut actual_bus, &mut reference_bus] {
                assert!(bus.ram.write_u32(0x20000100, 0xa5a55a5a));
                assert!(bus.ram.write_u32(0x20000200, 0x12345678));
            }
            let retired = actual.run_t16_cached_run(&mut actual_bus, 1);
            assert!(retired <= 1);
            if retired == 1 {
                reference
                    .step_internal(&mut reference_bus, &[], &config)
                    .unwrap();
            }
            assert_eq!(
                serde_json::to_value(actual.snapshot()).unwrap(),
                serde_json::to_value(reference.snapshot()).unwrap(),
                "opcode={opcode:04x} flags={flags:08x} retired={retired}"
            );
            assert_eq!(
                actual_bus.ram.data, reference_bus.ram.data,
                "opcode={opcode:04x} flags={flags:08x}"
            );
            assert_eq!(
                actual_bus.access_counts(),
                reference_bus.access_counts(),
                "opcode={opcode:04x} flags={flags:08x}"
            );
        }
    }
}

#[test]
fn cached_scalar_declines_budget_sleep_width_tag_mmio_and_unmapped_without_side_effects() {
    for case in 0..8 {
        let (mut cpu, mut bus) = dispatch_fixture(&[0x6808], 0, false);
        let mut budget = 1;
        match case {
            0 => budget = 0,
            1 => cpu.sleeping = true,
            2 => cpu.waiting_for_event = true,
            3 => cpu.decode_cache[0x80] = None,
            4 => cpu.decode_cache[0x80].as_mut().unwrap().tag = 0x2100,
            5 => cpu.decode_cache[0x80].as_mut().unwrap().pc_increment = 4,
            6 => cpu.r1 = 0x40003104,
            _ => cpu.r1 = 0xffffffff,
        }
        let before = serde_json::to_value(cpu.snapshot()).unwrap();
        let ram = bus.ram.data.clone();
        let counts = bus.access_counts();
        assert_eq!(cpu.run_t16_cached_run(&mut bus, budget), 0, "case={case}");
        assert_eq!(
            serde_json::to_value(cpu.snapshot()).unwrap(),
            before,
            "case={case}"
        );
        assert_eq!(bus.ram.data, ram, "case={case}");
        assert_eq!(bus.access_counts(), counts, "case={case}");
    }
}

#[test]
fn cached_runs_match_interpreter_across_budgets_branches_and_live_ram() {
    for (case, ops) in [
        &[0xbf00; 40][..],
        &[0x2001, 0x3001, 0x6008, 0x680a, 0xbf00],
        &[0x2003, 0x3801, 0xd1fd, 0xbf00, 0xbf30],
        &[0xbf00, 0xe7fd],
        &[0x3001, 0x6808, 0xbf00],
        &[0x3001, 0xbf08, 0xbf00],
        &[0x3001, 0xbf30, 0xbf00],
        &[0x3001, 0x6008, 0x680a, 0xe7fb],
    ]
    .into_iter()
    .enumerate()
    {
        for budget in (0..=33).chain([64, u32::MAX]) {
            for mmio in [false, true] {
                let (mut actual, mut bus) = dispatch_fixture(ops, 0, mmio);
                let (mut reference, mut reference_bus) = dispatch_fixture(ops, 0, mmio);
                let retired = actual.run_t16_cached_run(&mut bus, budget);
                assert!(retired <= budget.min(16));
                let available = match case {
                    0 | 3 => 16,
                    1 => {
                        if mmio {
                            2
                        } else {
                            5
                        }
                    }
                    2 => 8,
                    4 => {
                        if mmio {
                            1
                        } else {
                            3
                        }
                    }
                    5 | 6 => 1,
                    7 => {
                        if mmio {
                            1
                        } else {
                            16
                        }
                    }
                    _ => unreachable!(),
                };
                assert_eq!(
                    retired,
                    budget.min(16).min(available),
                    "case={case} budget={budget} mmio={mmio}"
                );
                let config = reference_bus.config.clone();
                for _ in 0..retired {
                    reference
                        .step_internal(&mut reference_bus, &[], &config)
                        .unwrap();
                }
                assert_eq!(
                    serde_json::to_value(actual.snapshot()).unwrap(),
                    serde_json::to_value(reference.snapshot()).unwrap(),
                    "ops={ops:x?} budget={budget} mmio={mmio}"
                );
                assert_eq!(bus.ram.data, reference_bus.ram.data);
                assert_eq!(bus.access_counts(), reference_bus.access_counts());
            }
        }
    }
}

#[test]
fn cached_runs_stop_before_cold_collision_wide_and_dynamic_mmio_barriers() {
    for case in 0..5 {
        let (mut cpu, mut bus) = dispatch_fixture(&[0x3001, 0x6808, 0xbf00], 0, false);
        match case {
            0 => cpu.decode_cache[0x81] = None,
            1 => cpu.decode_cache[0x81].as_mut().unwrap().tag = 0x2102,
            2 => cpu.decode_cache[0x81].as_mut().unwrap().pc_increment = 4,
            3 => cpu.r1 = 0x40003104,
            _ => cpu.r1 = 0xffffffff,
        }
        let ram = bus.ram.data.clone();
        let counts = bus.access_counts();
        assert_eq!(cpu.run_t16_cached_run(&mut bus, 16), 1, "case={case}");
        assert_eq!(cpu.pc, 0x102);
        assert_eq!(cpu.r0, 51);
        assert_eq!(bus.ram.data, ram);
        assert_eq!(bus.access_counts(), counts);
    }
}

// Frozen pre-dispatch call order: selected paths must retire the same work,
// retain the same state and preserve RAM/MMIO access accounting.
fn original_fast_paths(cpu: &mut CortexM, bus: &mut SystemBus, budget: u32) -> u32 {
    let mut retired = cpu.run_t16_self_branch(budget);
    if retired == 0 {
        retired = cpu.run_t16_countdown(budget);
    }
    if retired == 0 {
        retired = cpu.run_t16_store_spin(bus, budget);
    }
    if retired == 0 {
        retired = cpu.run_t16_ram_fast(bus, budget, true);
    }
    if retired == 0 {
        retired = cpu.run_t16_fast_block(bus, budget);
    }
    retired
}

fn dispatch_fixture(ops: &[u16], phase: usize, mmio: bool) -> (CortexM, SystemBus) {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    cpu.pc = 0x100 + phase as u32 * 2;
    cpu.r0 = 50;
    cpu.r1 = if mmio { 0x40003104 } else { 0x20000100 };
    cpu.sp = 0x20000200;
    assert!(bus.ram.write_u32(0x20000100, 0xdeadbeef));
    for (i, &op) in ops.iter().enumerate() {
        let pc = 0x100 + i as u32 * 2;
        cache(&mut cpu, pc, op);
        assert!(bus.flash.write_u16(u64::from(pc), op));
    }
    (cpu, bus)
}

#[test]
fn opcode_dispatch_matches_original_paths_at_rotated_entries_and_budgets() {
    for ops in [
        &[0xe7fe][..],                     // B .
        &[0x3801, 0xd1fd],                 // Countdown.
        &[0x3001, 0x6008, 0x680a, 0xe7fb], // RAM add/store/load loop.
        &[0x9000, 0x4669, 0x1c40, 0xe7fb], // Stack store/MOV/ADD loop.
        &[0x2001, 0x3001, 0xe7fc],         // General ALU block.
        &[0x6808, 0xe7fd],                 // RAM/MMIO-dependent block admission.
        &[0xbf30, 0xe7fd],                 // Unsupported WFI must decline.
    ] {
        for phase in 0..ops.len() {
            for budget in (0..=17).chain([65, 128]) {
                for mmio in [false, true] {
                    let (mut actual, mut actual_bus) = dispatch_fixture(ops, phase, mmio);
                    let (mut original, mut original_bus) = dispatch_fixture(ops, phase, mmio);
                    let retired = actual.run_t16_cached_fast_paths(&mut actual_bus, budget);
                    let expected = original_fast_paths(&mut original, &mut original_bus, budget);
                    assert_eq!(
                        retired, expected,
                        "ops={ops:x?} phase={phase} budget={budget} mmio={mmio}"
                    );
                    assert_eq!(
                        serde_json::to_value(actual.snapshot()).unwrap(),
                        serde_json::to_value(original.snapshot()).unwrap(),
                        "ops={ops:x?} phase={phase} budget={budget} mmio={mmio}"
                    );
                    assert_eq!(actual_bus.ram.data, original_bus.ram.data);
                    assert_eq!(actual_bus.access_counts(), original_bus.access_counts());
                }
            }
        }
    }
}

#[test]
fn dispatch_declines_cold_collided_and_wide_entries_without_side_effects() {
    for kind in 0..3 {
        let (mut cpu, mut bus) = dispatch_fixture(&[0xe7fe], 0, false);
        match kind {
            0 => cpu.decode_cache[0x80] = None,
            1 => cpu.decode_cache[0x80].as_mut().unwrap().tag = 0x2100,
            _ => cpu.decode_cache[0x80].as_mut().unwrap().pc_increment = 4,
        }
        let before = serde_json::to_value(cpu.snapshot()).unwrap();
        let counts = bus.access_counts();
        assert_eq!(cpu.run_t16_cached_fast_paths(&mut bus, 128), 0);
        assert_eq!(serde_json::to_value(cpu.snapshot()).unwrap(), before);
        assert_eq!(bus.access_counts(), counts);
    }
}

#[test]
fn borrowed_cached_opcode_preserves_every_halfword_and_width_filter() {
    let mut cpu = CortexM::new();
    for opcode in 0..=u16::MAX {
        cache(&mut cpu, 0x100, opcode);
        assert_eq!(cpu.cached_t16(0x100), Some(opcode));
        assert_eq!(cpu.cached_t16(0x2100), None);
        cpu.decode_cache[0x80].as_mut().unwrap().pc_increment = 4;
        assert_eq!(cpu.cached_t16(0x100), None);
    }
}

fn cache(cpu: &mut CortexM, pc: u32, opcode: u16) {
    cpu.insert_decoded_entry(DecodeCacheEntry {
        tag: pc,
        instruction: decode_thumb_16(opcode),
        opcode: u32::from(opcode),
        pc_increment: 2,
        cycles: 1,
    });
}

fn loop_fixture(start: u32) -> (CortexM, SystemBus) {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    cpu.pc = start;
    cpu.r0 = 50;
    // SUBS r0,#1; BNE start. PC-relative displacement at BNE is -6.
    for (i, op) in [0x3801, 0xd1fd].into_iter().enumerate() {
        let pc = start + i as u32 * 2;
        cache(&mut cpu, pc, op);
        assert!(bus.flash.write_u16(u64::from(pc), op));
    }
    (cpu, bus)
}

#[test]
fn cold_miss_is_not_a_permanent_negative_cache() {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    cpu.pc = 0x100;
    cpu.r0 = 3;
    assert!(!cpu.t16_block_entry_admitted(cpu.pc));
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 0);
    cache(&mut cpu, 0x100, 0x3801);
    cache(&mut cpu, 0x102, 0xd1fd);
    assert!(cpu.t16_block_entry_admitted(cpu.pc));
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
    assert_eq!((cpu.pc, cpu.r0), (0x100, 2));
}

#[test]
fn rotated_entry_and_branch_entry_match_reference_for_every_budget() {
    for phase in [0, 2] {
        for budget in 1..=15 {
            let (mut fast, mut fast_bus) = loop_fixture(0x100);
            let (mut reference, mut reference_bus) = loop_fixture(0x100);
            fast.pc += phase;
            reference.pc += phase;
            // The rotated branch starts with Z clear, so BNE is taken.
            fast.xpsr &= !(1 << 30);
            reference.xpsr &= !(1 << 30);
            assert!(fast.t16_block_entry_admitted(fast.pc));
            let retired = fast.run_t16_fast_block(&mut fast_bus, budget);
            assert_eq!(retired, budget, "phase={phase} budget={budget}");
            let config = reference_bus.config.clone();
            for _ in 0..retired {
                reference
                    .step_internal(&mut reference_bus, &[], &config)
                    .unwrap();
            }
            assert_eq!(fast.pc, reference.pc, "phase={phase} budget={budget}");
            assert_eq!(fast.r0, reference.r0);
            assert_eq!(fast.xpsr, reference.xpsr);
        }
    }
}

#[test]
fn cbnz_is_rejected_at_current_entry_and_cannot_be_spanned() {
    let (mut cpu, mut bus) = loop_fixture(0x100);
    cache(&mut cpu, 0x104, 0xb900); // CBNZ r0,+0
    assert!(matches!(
        cpu.decode_cache[0x82].unwrap().instruction,
        Instruction::Cbnz { .. }
    ));
    cpu.pc = 0x104;
    assert!(!cpu.t16_block_entry_admitted(cpu.pc));
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 64), 0);
    assert!(cpu.t16_fast_block.is_none());
    cache(&mut cpu, 0x106, 0x3801);
    cache(&mut cpu, 0x108, 0xd1fa); // BNE 0x100, across CBNZ
    cpu.pc = 0x106;
    assert!(cpu.t16_block_entry_admitted(cpu.pc));
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 64), 0);
    assert_eq!((cpu.pc, cpu.r0), (0x106, 50));
}

#[test]
fn thumb32_width_is_a_barrier_even_if_instruction_is_supported() {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    cache(&mut cpu, 0x100, 0x3801);
    cache(&mut cpu, 0x102, 0xbf00);
    cpu.decode_cache[0x81].as_mut().unwrap().pc_increment = 4;
    cpu.pc = 0x102;
    assert!(!cpu.t16_block_entry_admitted(cpu.pc));
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 64), 0);
    // A real Thumb32 occupies both halfwords; no entry is fabricated at +2.
    cache(&mut cpu, 0x106, 0x3801);
    cache(&mut cpu, 0x108, 0xd1fa);
    cpu.pc = 0x106;
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 64), 0);
    assert!(cpu.t16_fast_block.is_none());
}

#[test]
fn invalid_decode_tag_does_not_admit_aliasing_slot() {
    let (mut cpu, mut bus) = loop_fixture(0x100);
    cpu.decode_cache[0x80].as_mut().unwrap().tag = 0x2100;
    assert!(!cpu.t16_block_entry_admitted(0x100));
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 64), 0);
    assert_eq!((cpu.pc, cpu.r0), (0x100, 50));
    // Repairing the collision permits discovery immediately.
    cache(&mut cpu, 0x100, 0x3801);
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
}

#[test]
fn backward_search_at_zero_does_not_underflow_or_poison_future_discovery() {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    cpu.pc = 0;
    cache(&mut cpu, 0, 0xbf00);
    assert!(cpu.t16_block_entry_admitted(0));
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 64), 0);
    assert_eq!(cpu.pc, 0);
    let (mut cpu, mut bus) = loop_fixture(0);
    cpu.pc = 2;
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 3), 3);
    assert_eq!((cpu.pc, cpu.r0), (0, 49));
}

#[test]
fn admitted_load_falls_back_for_mmio_without_side_effects_then_accepts_ram() {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    cache(&mut cpu, 0x100, 0x6808); // LDR r0,[r1,#0]
    cache(&mut cpu, 0x102, 0xe7fd); // B 0x100
    cpu.pc = 0x100;
    cpu.r0 = 0xdeadbeef;
    cpu.r1 = 0x40003104; // TWIM event MMIO, never direct RAM
    assert!(cpu.t16_block_entry_admitted(cpu.pc));
    let before = bus.access_counts();
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 0);
    assert_eq!(bus.access_counts(), before);
    assert_eq!((cpu.pc, cpu.r0), (0x100, 0xdeadbeef));
    cpu.r1 = 0x20000100;
    assert!(bus.ram.write_u32(u64::from(cpu.r1), 0x12345678));
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
    assert_eq!((cpu.pc, cpu.r0), (0x100, 0x12345678));
    // A positive cache must still fall back when only the effective address
    // changes, and that execution failure must never become a discovery miss.
    cpu.r1 = 0x40003104;
    let before = (cpu.pc, cpu.r0, bus.access_counts());
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 0);
    assert_eq!((cpu.pc, cpu.r0, bus.access_counts()), before);
    assert_ne!(cpu.t16_discovery_misses[0], (cpu.pc, cpu.decode_generation));
    cpu.r1 = 0x20000100;
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
}

#[test]
fn cached_block_before_barrier_cannot_execute_unrelated_tail() {
    let (mut cpu, mut bus) = loop_fixture(0x100);
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
    assert!(cpu.t16_fast_block.is_some());
    cache(&mut cpu, 0x104, 0xb900);
    cache(&mut cpu, 0x106, 0x3801);
    cache(&mut cpu, 0x108, 0xd1fa);
    cpu.pc = 0x106;
    let before = cpu.r0;
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 64), 0);
    assert_eq!((cpu.pc, cpu.r0), (0x106, before));
}

#[test]
fn forward_and_wrong_target_terminal_branches_reject_without_retiring() {
    for terminal in [0xe001, 0xe7fc, 0xe7fe] {
        let (mut cpu, mut bus) = loop_fixture(0x100);
        cache(&mut cpu, 0x102, terminal);
        assert!(cpu.t16_block_entry_admitted(0x102));
        assert!(cpu.compile_t16_fast_block(0x100).is_none());
        let before = (cpu.pc, cpu.r0, cpu.xpsr, bus.access_counts());
        assert_eq!(cpu.run_t16_fast_block(&mut bus, 64), 0);
        assert_eq!((cpu.pc, cpu.r0, cpu.xpsr, bus.access_counts()), before);
        assert_eq!(cpu.t16_discovery_misses[0], (cpu.pc, cpu.decode_generation));
        assert_eq!(cpu.run_t16_fast_block(&mut bus, 64), 0);
        assert_eq!((cpu.pc, cpu.r0, cpu.xpsr, bus.access_counts()), before);
        cache(&mut cpu, 0x102, 0xd1fd);
        assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
        assert_eq!((cpu.pc, cpu.r0), (0x100, 49));
    }
}

#[test]
fn supported_window_longer_than_capacity_rejects_from_start_and_terminal() {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    for i in 0..T16_FAST_BLOCK_MAX {
        cache(&mut cpu, 0x100 + i as u32 * 2, 0xbf00);
    }
    // Sixteen NOPs followed by B 0x100: seventeen operations cannot fit.
    let terminal_pc = 0x100 + T16_FAST_BLOCK_MAX as u32 * 2;
    let displacement = (0x100_i32 - terminal_pc as i32 - 4) / 2;
    cache(
        &mut cpu,
        terminal_pc,
        0xe000 | ((displacement as u16) & 0x7ff),
    );
    assert!(cpu.compile_t16_fast_block(0x100).is_none());
    for pc in [0x100, 0x102, terminal_pc] {
        cpu.pc = pc;
        let before = (cpu.pc, cpu.r0, cpu.xpsr, bus.access_counts());
        assert_eq!(cpu.run_t16_fast_block(&mut bus, 64), 0);
        assert_eq!((cpu.pc, cpu.r0, cpu.xpsr, bus.access_counts()), before);
    }
}

#[test]
fn invalid_tag_inside_candidate_rejects_then_repaired_window_matches_interpreter() {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    cpu.pc = 0x100;
    cpu.r0 = 50;
    for (i, op) in [0x3801, 0xbf00, 0xd1fc].into_iter().enumerate() {
        let pc = 0x100 + i as u32 * 2;
        cache(&mut cpu, pc, op);
        assert!(bus.flash.write_u16(u64::from(pc), op));
    }
    cpu.decode_cache[0x81].as_mut().unwrap().tag = 0x2102;
    assert!(cpu.t16_block_entry_admitted(0x100));
    assert!(cpu.compile_t16_fast_block(0x100).is_none());
    let before = (cpu.pc, cpu.r0, cpu.xpsr, bus.access_counts());
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 64), 0);
    assert_eq!((cpu.pc, cpu.r0, cpu.xpsr, bus.access_counts()), before);
    cache(&mut cpu, 0x102, 0xbf00);
    let mut reference = CortexM::new();
    reference.pc = cpu.pc;
    reference.r0 = cpu.r0;
    reference.xpsr = cpu.xpsr;
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 12), 12);
    let config = bus.config.clone();
    for _ in 0..12 {
        reference.step_internal(&mut bus, &[], &config).unwrap();
    }
    assert_eq!(
        (cpu.pc, cpu.r0, cpu.xpsr),
        (reference.pc, reference.r0, reference.xpsr)
    );
}

#[test]
fn discovery_memo_is_stable_until_real_decoder_warms_the_window() {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    cpu.pc = 0x100;
    cpu.r0 = 4;
    assert!(bus.flash.write_u16(0x100, 0x3801));
    assert!(bus.flash.write_u16(0x102, 0xd1fd));
    let generation = cpu.decode_generation;
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 9), 0);
    assert_eq!(cpu.t16_discovery_misses[0], (0x100, generation));
    let before = (cpu.pc, cpu.r0, cpu.xpsr, bus.access_counts());
    for _ in 0..4 {
        assert_eq!(cpu.run_t16_fast_block(&mut bus, 9), 0);
    }
    assert_eq!((cpu.pc, cpu.r0, cpu.xpsr, bus.access_counts()), before);
    let config = bus.config.clone();
    for _ in 0..2 {
        cpu.step_internal(&mut bus, &[], &config).unwrap();
    }
    assert!(cpu.decode_generation > generation);
    assert_eq!((cpu.pc, cpu.r0), (0x100, 3));
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 4), 4);
    assert_eq!((cpu.pc, cpu.r0), (0x100, 1));
}

#[test]
fn fast_fetch_insertion_and_decode_tag_collision_invalidate_discovery_misses() {
    let (mut cpu, mut bus) = loop_fixture(0x100);
    cache(&mut cpu, 0x2100, 0xbf00); // same decode index, different PC tag
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 0);
    let generation = cpu.decode_generation;
    assert_eq!(cpu.fetch_t16_fast(&mut bus, 0x100, true), Some(0x3801));
    assert!(cpu.decode_generation > generation);
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
}

#[test]
fn memo_slot_aliases_require_exact_pc_and_generation() {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    cpu.pc = 0x100;
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 1), 0);
    cpu.pc = 0x180; // same bounded memo slot, not same PC
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 1), 0);
    assert_eq!(cpu.t16_discovery_misses[0], (0x180, cpu.decode_generation));
    cpu.pc = 0x100;
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 1), 0);
    assert_eq!(cpu.t16_discovery_misses[0], (0x100, cpu.decode_generation));
}

#[test]
fn invalidate_reset_and_snapshot_restore_drop_positive_and_negative_state() {
    for path in 0..3 {
        let (mut cpu, mut bus) = loop_fixture(0x100);
        assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
        assert!(cpu.t16_fast_block.is_some());
        cpu.pc = 0x200;
        assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 0);
        // Re-establish positive cache while retaining the unrelated miss.
        cpu.pc = 0x100;
        assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
        let generation = cpu.decode_generation;
        match path {
            0 => cpu.invalidate_code_caches(),
            1 => cpu.reset(&mut bus).unwrap(),
            _ => {
                let snapshot = cpu.snapshot();
                cpu.apply_snapshot(&snapshot);
            }
        }
        assert_ne!(cpu.decode_generation, generation);
        assert!(cpu.t16_fast_block.is_none());
        assert!(cpu.decode_cache.iter().all(Option::is_none));
        // Code can be patched into a supported loop after any flush.
        assert!(bus.flash.write_u16(0x100, 0x3802)); // patched SUBS r0,#2
        assert!(bus.flash.write_u16(0x102, 0xd1fd));
        cpu.pc = 0x100;
        cpu.r0 = 8;
        let config = bus.config.clone();
        for _ in 0..2 {
            cpu.step_internal(&mut bus, &[], &config).unwrap();
        }
        assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
        assert_eq!((cpu.pc, cpu.r0), (0x100, 4));
    }
}

#[test]
fn generation_wrap_clears_old_misses_before_reusing_generation_one() {
    let mut cpu = CortexM::new();
    cpu.t16_discovery_misses.fill((0x100, 1));
    cpu.decode_generation = u64::MAX;
    cache(&mut cpu, 0x100, 0xbf00);
    assert_eq!(cpu.decode_generation, 1);
    assert!(cpu
        .t16_discovery_misses
        .iter()
        .all(|entry| *entry == (0, 0)));
}

#[test]
fn positive_cache_is_considered_before_negative_memo_and_zero_budget_retires_nothing() {
    let (mut cpu, mut bus) = loop_fixture(0x100);
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
    cpu.memoize_t16_discovery_miss();
    let before = (cpu.pc, cpu.r0, cpu.xpsr, bus.access_counts());
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 0), 0);
    assert_eq!((cpu.pc, cpu.r0, cpu.xpsr, bus.access_counts()), before);
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
    assert_eq!((cpu.pc, cpu.r0), (0x100, 48));
}

#[test]
fn decoding_disabled_then_enabled_warms_cache_and_invalidates_old_miss() {
    let mut cpu = CortexM::new();
    let mut bus = SystemBus::new();
    cpu.pc = 0x100;
    cpu.r0 = 5;
    assert!(bus.flash.write_u16(0x100, 0x3801));
    assert!(bus.flash.write_u16(0x102, 0xd1fd));
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 0);
    let generation = cpu.decode_generation;
    let mut config = bus.config.clone();
    config.decode_cache_enabled = false;
    for _ in 0..2 {
        cpu.step_internal(&mut bus, &[], &config).unwrap();
    }
    assert_eq!((cpu.pc, cpu.r0), (0x100, 4));
    assert_eq!(cpu.decode_generation, generation);
    assert!(cpu.decoded_entry(0x100).is_none());
    config.decode_cache_enabled = true;
    for _ in 0..2 {
        cpu.step_internal(&mut bus, &[], &config).unwrap();
    }
    assert!(cpu.decode_generation > generation);
    assert_eq!(cpu.run_t16_fast_block(&mut bus, 2), 2);
    assert_eq!((cpu.pc, cpu.r0), (0x100, 2));
}
