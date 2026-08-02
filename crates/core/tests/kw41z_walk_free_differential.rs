// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Walk-free **kw41z** batch (mcg / rsim / dwt → walk-independent), in the
//! `stm32_timer_walk_differential` (B2/B3) style: the SAME hand-built Cortex-M
//! machine and hand-assembled Thumb firmware run twice — once with DWT pinned
//! back onto the per-cycle walk (`force_legacy_walk`, the reference) and once
//! scheduler-driven (lazy CYCCNT) — and every observable is compared. Plus the
//! inertness proof for the two combinational Kinetis models and the
//! board-config flip status.
//!
//! ## Gates
//!
//! 1. `dwt_cyccnt_is_byte_identical_at_interval_1` — firmware enables
//!    `DWT_CTRL.CYCCNTENA`, then polls `DWT_CYCCNT` from its main loop (the
//!    lazy-read surface) at many distinct, unaligned cycle counts. Probed after
//!    EVERY instruction: `total_cycles`, PC, all 16 core registers, and the
//!    stored CYCCNT sample must be byte-identical between the walk reference and
//!    the walk-deleted scheduler lane. This is the "a read at any cycle returns
//!    exactly what the walk would have produced" proof (the CYCCNT reads land at
//!    arbitrary unaligned cycles and every one is exact).
//!
//! 2. `dwt_cyccnt_reads_are_bounded_and_live_at_interval_64` — the same firmware
//!    with the scheduler lane batched at tick interval 64 vs the walk-on
//!    interval-1 golden reference. A bare Cortex-M + DWT fixture avoids the SCB
//!    reset-fidelity clamp, and the step profile proves that wide batches really
//!    formed. Mid-batch CYCCNT reads quantise to the batch-start grid, so every
//!    read stays within one interval of the walk reference and the trace stays
//!    live. `total_cycles` remains identical.
//!
//! 3. `mcg_tick_is_a_genuine_no_op` / `rsim_tick_is_a_genuine_no_op` — the two
//!    combinational Kinetis models are driven through their real boot register
//!    sequences and then ticked; the snapshot is byte-identical before and after
//!    any number of ticks and the tick result is `default()`. This is the
//!    inertness proof behind their `needs_legacy_walk() == false`.
//!
//! 4. `kw41z_board_walk_forcing_set_is_empty` /
//!    `kw41z_board_derive_walk_deletable_flips` — the board-config flip status
//!    on the honestly-assembled FRDM-KW41Z bus (`from_config` +
//!    `configure_cortex_m`, exactly how the run path builds it). With the shared
//!    Kinetis I2C `i2c1` migrated in batch B4 (held-level re-pend event chain),
//!    the walk-forcing set is now EMPTY and the bus derives walk-deletion with
//!    no hand flag — the campaign's first full-board flip.

#![cfg(feature = "event-scheduler")]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::peripherals::dwt::Dwt;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::{DebugControl, Machine, Peripheral};

// ── DWT differential (hand-built Cortex-M) ──────────────────────────────────

const DWT_BASE: u64 = 0xE000_1000;
const MAIN_COUNT_ADDR: u64 = 0x2000_0004;
const CYCCNT_STORE_ADDR: u64 = 0x2000_0008;
const INITIAL_SP: u32 = 0x2000_8000;

fn load_thumb(bus: &mut SystemBus, base: u64, halfwords: &[u16]) {
    for (i, hw) in halfwords.iter().enumerate() {
        bus.write_u16(base + (i as u64) * 2, *hw).unwrap();
    }
}

/// Firmware — enable CYCCNT (DWT_CTRL.CYCCNTENA), then spin: increment the main
/// counter, poll `DWT_CYCCNT` (the lazy-read surface), and store the sample.
///
///   0x40: 4806  ldr r0, [pc, #24]   ; = DWT_BASE          (pool at 0x5C)
///   0x42: 2101  movs r1, #1
///   0x44: 6001  str r1, [r0, #0]    ; DWT_CTRL = CYCCNTENA
///   0x46: 4A06  ldr r2, [pc, #24]   ; = MAIN_COUNT_ADDR   (pool at 0x60)
///   0x48: 4D06  ldr r5, [pc, #24]   ; = CYCCNT_STORE_ADDR (pool at 0x64)
///   0x4A: 2300  movs r3, #0
///   loop:
///   0x4C: 3301  adds r3, #1
///   0x4E: 6013  str r3, [r2]        ; main_count++
///   0x50: 6844  ldr r4, [r0, #4]    ; poll CYCCNT (lazy read)
///   0x52: 602C  str r4, [r5]        ; store last sample
///   0x54: E7FA  b loop
fn load_firmware_cyccnt_poll(bus: &mut SystemBus) {
    load_thumb(
        bus,
        0x40,
        &[
            0x4806, 0x2101, 0x6001, 0x4A06, 0x4D06, 0x2300, 0x3301, 0x6013, 0x6844, 0x602C, 0xE7FA,
            0xBF00, 0xBF00, 0xBF00,
        ],
    );
    bus.write_u32(0x5C, DWT_BASE as u32).unwrap();
    bus.write_u32(0x60, MAIN_COUNT_ADDR as u32).unwrap();
    bus.write_u32(0x64, CYCCNT_STORE_ADDR as u32).unwrap();
}

/// Base of the CYCCNT sample buffer written by `load_firmware_cyccnt_buffer`.
const BUFFER_BASE: u64 = 0x2000_0100;

/// Firmware variant for the batched interval-64 gate — same enable, but stores
/// each `DWT_CYCCNT` sample into a rolling buffer (`r5` bumped by 4 each pass)
/// so a single batched run leaves a full trace of mid-batch reads to inspect.
///
///   0x40: 4804  ldr r0, [pc, #16]   ; = DWT_BASE     (pool at 0x54)
///   0x42: 2101  movs r1, #1
///   0x44: 6001  str r1, [r0, #0]    ; DWT_CTRL = CYCCNTENA
///   0x46: 4D04  ldr r5, [pc, #16]   ; = BUFFER_BASE  (pool at 0x58)
///   loop:
///   0x48: 6844  ldr r4, [r0, #4]    ; poll CYCCNT (lazy read)
///   0x4A: 602C  str r4, [r5]        ; *r5 = CYCCNT
///   0x4C: 3504  adds r5, #4         ; r5 += 4
///   0x4E: E7FB  b loop
fn load_firmware_cyccnt_buffer(bus: &mut SystemBus) {
    load_thumb(
        bus,
        0x40,
        &[
            0x4804, 0x2101, 0x6001, 0x4D04, 0x6844, 0x602C, 0x3504, 0xE7FB, 0xBF00, 0xBF00,
        ],
    );
    bus.write_u32(0x54, DWT_BASE as u32).unwrap();
    bus.write_u32(0x58, BUFFER_BASE as u32).unwrap();
}

/// `SystemBus::new()` + `configure_cortex_m` (real SCB/NVIC/DWT, DWT with the
/// bus cycle clock attached). `scheduler = false` pins DWT back onto the legacy
/// walk (`force_legacy_walk`) to form the reference lane from the same assembly.
fn build_machine(scheduler: bool, tick_interval: u32) -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);

    if !scheduler {
        let idx = bus.find_peripheral_index_by_name("dwt").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Dwt>()
            .unwrap()
            .force_legacy_walk();
    }

    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    machine.cpu.sp = INITIAL_SP;
    machine
}

/// Bare Cortex-M + DWT fixture for the wide-batch gate. Omitting SCB is
/// intentional: an attached SCB activates reset-fidelity planning, which
/// clamps batches to one instruction. The DWT still receives the real shared
/// bus cycle clock through `add_peripheral`, so this isolates the batched lazy
/// clock behavior the gate is meant to prove.
fn build_batchable_dwt_machine(scheduler: bool, tick_interval: u32) -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    bus.add_peripheral("dwt", DWT_BASE, 0x1000, None, Box::new(Dwt::new()));

    if !scheduler {
        let idx = bus.find_peripheral_index_by_name("dwt").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Dwt>()
            .unwrap()
            .force_legacy_walk();
    }

    let mut machine = Machine::new(CortexM::new(), bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    machine.cpu.sp = INITIAL_SP;
    machine
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    step: u64,
    total_cycles: u64,
    pc: u32,
    regs: [u32; 16],
    main_count: u32,
    cyccnt_sample: u32,
}

fn probe(machine: &Machine<CortexM>, step: u64) -> Probe {
    let mut regs = [0u32; 16];
    for (i, r) in regs.iter_mut().enumerate() {
        *r = machine.read_core_reg(i as u8);
    }
    Probe {
        step,
        total_cycles: machine.total_cycles,
        pc: machine.get_pc(),
        regs,
        main_count: machine.bus.read_u32(MAIN_COUNT_ADDR).unwrap(),
        cyccnt_sample: machine.bus.read_u32(CYCCNT_STORE_ADDR).unwrap(),
    }
}

/// Run `steps` instructions one at a time through the batched `Machine::run`
/// path, probing the full architectural state after every instruction.
fn run_probed(machine: &mut Machine<CortexM>, entry: u32, steps: u64) -> Vec<Probe> {
    machine.cpu.pc = entry;
    let mut probes = Vec::with_capacity(steps as usize);
    for s in 0..steps {
        machine.run(Some(1)).unwrap();
        probes.push(probe(machine, s + 1));
    }
    probes
}

/// Gate 1: lazy CYCCNT reads at unaligned cycles, walk-on vs walk-deleted
/// scheduler at tick interval 1 — every instruction-boundary observable
/// byte-identical (r4 carries the raw CYCCNT poll, stored to RAM).
#[test]
fn dwt_cyccnt_is_byte_identical_at_interval_1() {
    const STEPS: u64 = 3_000;

    let mut walk = build_machine(false, 1);
    load_firmware_cyccnt_poll(&mut walk.bus);
    let walk_probes = run_probed(&mut walk, 0x40, STEPS);

    let mut sched = build_machine(true, 1);
    load_firmware_cyccnt_poll(&mut sched.bus);
    let sched_probes = run_probed(&mut sched, 0x40, STEPS);

    assert_eq!(walk_probes.len(), sched_probes.len());
    for (r, c) in walk_probes.iter().zip(sched_probes.iter()) {
        assert_eq!(
            r, c,
            "DWT CYCCNT firmware: first divergence at step {} (walk-reference vs scheduler)",
            r.step
        );
    }

    let last = walk_probes.last().unwrap();
    assert!(
        last.cyccnt_sample > 1_000,
        "reference must observe a live, advancing CYCCNT (got {})",
        last.cyccnt_sample
    );
    // The reads land at many distinct cycle values (unaligned), so this is the
    // "exact at arbitrary cycles" proof, not just at a single point.
    let distinct: std::collections::BTreeSet<u32> =
        walk_probes.iter().map(|p| p.cyccnt_sample).collect();
    assert!(
        distinct.len() > 100,
        "CYCCNT must be sampled at many distinct cycle values (got {})",
        distinct.len()
    );
}

/// Gate 2: scheduler @ interval 64 vs walk-on interval-1 golden reference, run
/// as a SINGLE batched call each. Both lanes use the bare Cortex-M + DWT fixture
/// so SCB reset fidelity cannot clamp them to one instruction. The scheduler
/// step profile proves wide batches formed (`cpu_batches` is nonzero and
/// strictly less than `cpu_instructions`); its CYCCNT staircase independently
/// shows batch-start quantisation. Every sample stays within one interval of the
/// walk reference, the trace stays live, and `total_cycles` stays identical.
#[test]
fn dwt_cyccnt_reads_are_bounded_and_live_at_interval_64() {
    const STEPS: u64 = 4_000;
    const INTERVAL: u64 = 64;
    // Well under the ~999 samples STEPS produces (loop is 4 instructions), so
    // both lanes have written every slot we read.
    const N_SAMPLES: u64 = 800;

    let mut walk = build_batchable_dwt_machine(false, 1);
    load_firmware_cyccnt_buffer(&mut walk.bus);
    walk.cpu.pc = 0x40;
    walk.run(Some(STEPS as u32)).unwrap();

    let mut sched = build_batchable_dwt_machine(true, INTERVAL as u32);
    load_firmware_cyccnt_buffer(&mut sched.bus);
    sched.cpu.pc = 0x40;
    sched.run(Some(STEPS as u32)).unwrap();

    assert_eq!(
        walk.total_cycles, sched.total_cycles,
        "total_cycles must match (no timing artifact from the migration)"
    );
    let profile = sched.step_profile();
    assert_eq!(
        profile.cpu_instructions, STEPS,
        "the single scheduler run must retire the requested instruction budget"
    );
    assert!(
        profile.cpu_batches > 0,
        "the scheduler run must execute at least one CPU batch"
    );
    assert!(
        profile.cpu_batches < profile.cpu_instructions,
        "interval-64 batching must be non-vacuous (batches={}, instructions={})",
        profile.cpu_batches,
        profile.cpu_instructions
    );

    let read_buf = |m: &Machine<CortexM>| -> Vec<u32> {
        (0..N_SAMPLES)
            .map(|i| m.bus.read_u32(BUFFER_BASE + i * 4).unwrap())
            .collect()
    };
    let w = read_buf(&walk);
    let s = read_buf(&sched);

    let mut saw_plateau = false;
    for i in 0..N_SAMPLES as usize {
        // Both traces are monotone non-decreasing (CYCCNT never rewinds).
        if i > 0 {
            assert!(w[i] >= w[i - 1], "walk CYCCNT rewound at sample {i}");
            assert!(s[i] >= s[i - 1], "scheduler CYCCNT rewound at sample {i}");
            assert!(
                w[i] > w[i - 1],
                "walk CYCCNT must strictly advance at sample {i}"
            );
            if s[i] == s[i - 1] {
                saw_plateau = true;
            }
        }
        let diff = (w[i] as i64 - s[i] as i64).unsigned_abs();
        assert!(
            diff < INTERVAL,
            "sample {i}: |walk {} - sched {}| = {} exceeds interval {}",
            w[i],
            s[i],
            diff,
            INTERVAL
        );
    }

    assert!(
        saw_plateau,
        "interval-64 CYCCNT must show batch-start quantisation"
    );
    // Live, not frozen: the scheduler counter climbs well past zero.
    assert!(
        *s.last().unwrap() > 1_000,
        "scheduler CYCCNT must keep counting true cycles (got {})",
        s.last().unwrap()
    );
}

// ── mcg / rsim inertness (the walk contributes nothing) ─────────────────────

/// Assert a tick result carries no side effect (no IRQ / DMA / exception / mmio
/// write / fired event / tick cost) — a genuine walk no-op. (`PeripheralTickResult`
/// does not derive `PartialEq`, so the fields are checked individually.)
fn assert_tick_is_inert(r: &labwired_core::PeripheralTickResult, what: &str) {
    assert!(!r.irq, "{what}: tick raised an IRQ");
    assert_eq!(r.cycles, 0, "{what}: tick charged cycles");
    assert!(
        r.dma_requests.is_none(),
        "{what}: tick emitted a DMA request"
    );
    assert!(
        r.explicit_irqs.is_none(),
        "{what}: tick raised explicit IRQs"
    );
    assert!(
        r.system_exception.is_none(),
        "{what}: tick raised a system exception"
    );
    assert!(r.dma_signals.is_none(), "{what}: tick emitted a DMA signal");
    assert!(
        r.mmio_writes.is_empty(),
        "{what}: tick requested an mmio write"
    );
    assert!(r.fired_events.is_empty(), "{what}: tick fired an event");
}

/// Drive the MCG through the NXP `CLOCK_SetFeeMode` control writes, then prove
/// its `tick()` is a genuine no-op: the snapshot is byte-identical before and
/// after any number of ticks, and the tick result is `default()`. This is the
/// evidence behind `Mcg::needs_legacy_walk() == false`.
#[test]
fn mcg_tick_is_a_genuine_no_op() {
    use labwired_core::peripherals::mcg::Mcg;

    let mut mcg = Mcg::new();
    // A representative spread of reachable control states (reset, FEE, external
    // clock select, crystal-osc enable).
    for &(off, val) in &[(0x00u64, 0x28u8), (0x01, 0xC4), (0x03, 0x20), (0x00, 0x80)] {
        mcg.write(off, val).unwrap();
        let before = mcg.snapshot();
        for _ in 0..1000 {
            assert_tick_is_inert(&mcg.tick(), "MCG");
        }
        // `tick_elapsed(N)` is the interval-batched walk entry — also inert.
        let _ = mcg.tick_elapsed(64);
        let after = mcg.snapshot();
        assert_eq!(
            before, after,
            "MCG state changed under the walk (state after write off={off:#x} val={val:#x})"
        );
    }
}

/// Drive the RSIM through `BOARD_RfOscInit` (enable the RF oscillator), then
/// prove its `tick()` is a genuine no-op — the evidence behind
/// `Rsim::needs_legacy_walk() == false`.
#[test]
fn rsim_tick_is_a_genuine_no_op() {
    use labwired_core::peripherals::rsim::Rsim;

    let mut rsim = Rsim::new();
    // Not-yet-enabled and enabled states both must be walk-inert.
    for enable in [false, true] {
        if enable {
            let ctrl = rsim.read_u32(0x000).unwrap();
            rsim.write_u32(0x000, (ctrl & !(0xF << 8)) | (1 << 8))
                .unwrap();
        }
        let before = rsim.snapshot();
        for _ in 0..1000 {
            assert_tick_is_inert(&rsim.tick(), "RSIM");
        }
        let _ = rsim.tick_elapsed(64);
        let after = rsim.snapshot();
        assert_eq!(
            before, after,
            "RSIM state changed under the walk (enable={enable})"
        );
    }
}

// ── kw41z board-config flip status ──────────────────────────────────────────

/// Assemble the FRDM-KW41Z bus exactly as the run path builds it: `from_config`
/// (chip + system manifest, any hand `walk_deleted` stripped) + the real
/// Cortex-M SCB / NVIC / DWT via `configure_cortex_m`.
fn frdm_kw41z_runtime_bus() -> SystemBus {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;

    let system_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/systems/frdm-kw41z.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load frdm-kw41z manifest");
    // Derive from the models, not the manifest escape hatch.
    manifest.walk_deleted = None;
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load mkw41z4 chip");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build frdm-kw41z bus");
    let _ = configure_cortex_m(&mut bus);
    bus
}

/// The exact `SystemBus::derive_walk_deletable` predicate (which is `pub(crate)`),
/// recomputed here from PUBLIC trait methods over the PUBLIC `peripherals` list:
/// walk-deletable iff EVERY peripheral is scheduler-driven or walk-independent.
fn is_walk_deletable(bus: &SystemBus) -> bool {
    bus.peripherals
        .iter()
        .all(|p| p.dev.uses_scheduler() || !p.dev.needs_legacy_walk())
}

fn walk_forcing_set(bus: &SystemBus) -> Vec<String> {
    let mut forcing: Vec<String> = bus
        .peripherals
        .iter()
        .filter(|p| p.dev.needs_legacy_walk() && !p.dev.uses_scheduler())
        .map(|p| p.name.clone())
        .collect();
    forcing.sort();
    forcing
}

/// After the mcg/rsim/dwt batch AND the shared-I2C (Kinetis) migration in batch
/// B4, NO peripheral on the kw41z bus forces the walk: `i2c1` is the Kinetis I2C
/// variant, whose `tick()` is a pure level-IRQ re-assertion now driven by the
/// held-level re-pend event chain, so it too leaves the walk-forcing set.
#[test]
fn kw41z_board_walk_forcing_set_is_empty() {
    let bus = frdm_kw41z_runtime_bus();
    let forcing = walk_forcing_set(&bus);
    assert!(
        forcing.is_empty(),
        "every kw41z walker must be migrated after B4 (i2c1 included); \
         remaining forcing set: {forcing:?}"
    );
}

/// The board flip: with `i2c1` (Kinetis) migrated, `derive_walk_deletable()`
/// returns true for the FRDM-KW41Z bus with no hand flag, making it the FIRST
/// board where the per-cycle walk is deleted for arbitrary firmware.
#[test]
fn kw41z_board_derive_walk_deletable_flips() {
    let bus = frdm_kw41z_runtime_bus();
    assert!(
        is_walk_deletable(&bus),
        "FRDM-KW41Z bus must derive walk-deletion once every walker (incl. i2c1) is \
         migrated; remaining forcing set: {:?}",
        walk_forcing_set(&bus)
    );
}
