// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential gates for walk-free batches **B2/B3** (STM32 TIMx → event
//! scheduler), in the `systick_walk_differential` (B1) style: the SAME
//! hand-built Cortex-M machine and hand-assembled Thumb firmware run twice —
//! once with the timer pinned back onto the per-cycle walk
//! (`force_legacy_walk`, the reference) and once scheduler-driven — and every
//! observable is compared.
//!
//! 1. `update_irq_firmware_is_byte_identical_at_interval_1` — a firmware that
//!    arms TIM (PSC/ARR/DIER.UIE/CR1.CEN), polls `CNT` from its main loop
//!    (the lazy-read surface), and takes the update IRQ through the NVIC
//!    (line 28 → exception 44). Probed after EVERY instruction: total_cycles,
//!    PC, all 16 core registers and both RAM counters must be byte-identical
//!    at tick interval 1 — which pins the NVIC IRQ delivery cycles (including
//!    the legacy level-repend double-entry artifact), the held-level counter
//!    freeze, and the lazy CNT reads exactly.
//!
//! 2. `compare_match_irq_firmware_is_byte_identical_at_interval_1` — the CCR1
//!    compare-match leg (DIER.CC1IE): the CCxIF latch lands on the match tick
//!    and the level pend on the NEXT tick, exactly like the walk.
//!
//! 3. `mid_run_reconfig_firmware_is_byte_identical_at_interval_1` — firmware
//!    rewrites PSC and ARR mid-count and issues a software update (EGR.UG):
//!    the model's immediate-apply prescaler semantics (NO buffered reload —
//!    a pinned limitation, silicon buffers PSC until the update event) and
//!    the pending-event cancel/re-arm on every write are held identical.
//!
//! 4. `cnt_sr_poll_firmware_is_byte_identical_at_interval_1` — the DIER=0
//!    polling shape (no freeze, no IRQs): raw CNT + lazily-latched SR reads
//!    every loop pass.
//!
//! 5. `wrap_count_is_exact_at_interval_64` — the scheduler lane at tick
//!    interval 64 vs the walk-on interval-1 golden reference, on the DIER=0
//!    UIF-poll-and-clear shape: lazy reads are quantised to the batch grid
//!    (documented, ≤ one interval), but the latched UIF can never be missed,
//!    so the observed update-event count over a fixed instruction window is
//!    EXACT (absolute closed-form deadlines — no cumulative drift), provided
//!    no wrap lands within one interval of the window edge (asserted from the
//!    reference's own trace).
//!
//!    The IRQ-enabled (UIE) shape is deliberately NOT count-compared at
//!    interval 64: the legacy level-repend during the ISR's flag-clear window
//!    (a faithful NVIC artifact, pinned byte-exact at interval 1 by gate 1)
//!    is sampled on the tick grid, so at interval 64 the mid-interval
//!    re-pends legitimately collapse — the same ≤-one-interval quantisation
//!    the write-path `sync_to` documents, and strictly better than the
//!    pre-migration walk at interval 64 (which slowed the whole counter 64×).

#![cfg(feature = "event-scheduler")]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::peripherals::timer::Timer;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::{DebugControl, Machine};

/// ISR increment target.
const ISR_COUNT_ADDR: u64 = 0x2000_0000;
/// Main-loop increment target.
const MAIN_COUNT_ADDR: u64 = 0x2000_0004;
const INITIAL_SP: u32 = 0x2000_8000;

const TIM_BASE: u32 = 0x4000_0000;
/// NVIC position for the hand-built TIM entry (TIM2 on STM32L4/F1) —
/// exception number 16 + 28 = 44, vector at 44*4 = 0xB0.
const TIM_IRQ: u32 = 28;
const NVIC_ISER0: u32 = 0xE000_E100;

fn load_thumb(bus: &mut SystemBus, base: u64, halfwords: &[u16]) {
    for (i, hw) in halfwords.iter().enumerate() {
        bus.write_u16(base + (i as u64) * 2, *hw).unwrap();
    }
}

fn write_word(bus: &mut SystemBus, addr: u64, word: u32) {
    bus.write_u32(addr, word).unwrap();
}

/// Build the shared machine assembly: `SystemBus::new()` +
/// `configure_cortex_m` (real SCB/NVIC/DWT) + a native `Timer` registered at
/// `TIM_BASE` with NVIC line 28 through `add_peripheral` — which attaches the
/// bus cycle clock exactly as the production `from_config` choke would.
/// `scheduler = false` then pins the timer back onto the legacy walk
/// (`force_legacy_walk`) to form the reference lane from the same assembly.
fn build_machine(scheduler: bool, tick_interval: u32) -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);

    bus.add_peripheral(
        "tim2",
        TIM_BASE as u64,
        0x400,
        Some(TIM_IRQ),
        Box::new(Timer::new()),
    );

    if !scheduler {
        let idx = bus.find_peripheral_index_by_name("tim2").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Timer>()
            .unwrap()
            .force_legacy_walk();
    }

    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    machine.cpu.sp = INITIAL_SP;
    machine
}

/// The shared ISR at 0x80: clear TIM SR (all flags), then increment the word
/// at `ISR_COUNT_ADDR` and return.
///
///   0x80: 4803  ldr r0, [pc, #12]   ; = TIM_BASE (pool at 0x90)
///   0x82: 2100  movs r1, #0
///   0x84: 6101  str r1, [r0, #0x10] ; SR = 0 (rc_w0 clears every flag)
///   0x86: 4803  ldr r0, [pc, #12]   ; = ISR_COUNT_ADDR (pool at 0x94)
///   0x88: 6801  ldr r1, [r0]
///   0x8A: 3101  adds r1, #1
///   0x8C: 6001  str r1, [r0]
///   0x8E: 4770  bx lr
///   0x90: .word TIM_BASE
///   0x94: .word ISR_COUNT_ADDR
const ISR_BASE: u64 = 0x80;
fn load_isr(bus: &mut SystemBus) {
    load_thumb(
        bus,
        ISR_BASE,
        &[
            0x4803, 0x2100, 0x6101, 0x4803, 0x6801, 0x3101, 0x6001, 0x4770,
        ],
    );
    write_word(bus, ISR_BASE + 0x10, TIM_BASE);
    write_word(bus, ISR_BASE + 0x14, ISR_COUNT_ADDR as u32);
    // Vector for exception 16 + TIM_IRQ = 44 → offset 0xB0.
    write_word(bus, (16 + TIM_IRQ) as u64 * 4, (ISR_BASE as u32) | 1);
}

/// Firmware A — arm the update interrupt: NVIC enable line 28, PSC=2, ARR=50,
/// DIER=UIE, CR1=CEN; then spin incrementing the main counter while polling
/// `CNT` (the lazy-read surface).
///
///   0x40: 4808  ldr r0, [pc, #32]   ; = NVIC_ISER0 (pool at 0x64)
///   0x42: 4909  ldr r1, [pc, #36]   ; = 1 << 28    (pool at 0x68)
///   0x44: 6001  str r1, [r0]
///   0x46: 4809  ldr r0, [pc, #36]   ; = TIM_BASE   (pool at 0x6C)
///   0x48: 2102  movs r1, #2
///   0x4A: 6281  str r1, [r0, #0x28] ; PSC = 2
///   0x4C: 2132  movs r1, #50
///   0x4E: 62C1  str r1, [r0, #0x2C] ; ARR = 50
///   0x50: 2101  movs r1, #1
///   0x52: 60C1  str r1, [r0, #0x0C] ; DIER = UIE
///   0x54: 6001  str r1, [r0, #0x00] ; CR1 = CEN
///   0x56: 4A06  ldr r2, [pc, #24]   ; = MAIN_COUNT_ADDR (pool at 0x70)
///   0x58: 2300  movs r3, #0
///   loop:
///   0x5A: 3301  adds r3, #1
///   0x5C: 6013  str r3, [r2]
///   0x5E: 6A44  ldr r4, [r0, #0x24] ; poll CNT (lazy read)
///   0x60: E7FB  b loop
///   0x62: BF00  nop (pool align)
fn load_firmware_update_irq(bus: &mut SystemBus) {
    load_thumb(
        bus,
        0x40,
        &[
            0x4808, 0x4909, 0x6001, 0x4809, 0x2102, 0x6281, 0x2132, 0x62C1, 0x2101, 0x60C1, 0x6001,
            0x4A06, 0x2300, 0x3301, 0x6013, 0x6A44, 0xE7FB, 0xBF00,
        ],
    );
    write_word(bus, 0x64, NVIC_ISER0);
    write_word(bus, 0x68, 1 << TIM_IRQ);
    write_word(bus, 0x6C, TIM_BASE);
    write_word(bus, 0x70, MAIN_COUNT_ADDR as u32);
    load_isr(bus);
}

/// Firmware B — the compare-match leg: CCR1=30, DIER=CC1IE, ARR=50, PSC=2.
///
///   0x40: 4809  ldr r0, [pc, #36]   ; = NVIC_ISER0 (pool at 0x68)
///   0x42: 490A  ldr r1, [pc, #40]   ; = 1 << 28    (pool at 0x6C)
///   0x44: 6001  str r1, [r0]
///   0x46: 480A  ldr r0, [pc, #40]   ; = TIM_BASE   (pool at 0x70)
///   0x48: 2102  movs r1, #2
///   0x4A: 6281  str r1, [r0, #0x28] ; PSC = 2
///   0x4C: 2132  movs r1, #50
///   0x4E: 62C1  str r1, [r0, #0x2C] ; ARR = 50
///   0x50: 211E  movs r1, #30
///   0x52: 6341  str r1, [r0, #0x34] ; CCR1 = 30
///   0x54: 2102  movs r1, #2
///   0x56: 60C1  str r1, [r0, #0x0C] ; DIER = CC1IE
///   0x58: 2101  movs r1, #1
///   0x5A: 6001  str r1, [r0, #0x00] ; CR1 = CEN
///   0x5C: 4A05  ldr r2, [pc, #20]   ; = MAIN_COUNT_ADDR (pool at 0x74)
///   0x5E: 2300  movs r3, #0
///   loop:
///   0x60: 3301  adds r3, #1
///   0x62: 6013  str r3, [r2]
///   0x64: 6A44  ldr r4, [r0, #0x24] ; poll CNT
///   0x66: E7FB  b loop
fn load_firmware_compare_irq(bus: &mut SystemBus) {
    load_thumb(
        bus,
        0x40,
        &[
            0x4809, 0x490A, 0x6001, 0x480A, 0x2102, 0x6281, 0x2132, 0x62C1, 0x211E, 0x6341, 0x2102,
            0x60C1, 0x2101, 0x6001, 0x4A05, 0x2300, 0x3301, 0x6013, 0x6A44, 0xE7FB,
        ],
    );
    write_word(bus, 0x68, NVIC_ISER0);
    write_word(bus, 0x6C, 1 << TIM_IRQ);
    write_word(bus, 0x70, TIM_BASE);
    write_word(bus, 0x74, MAIN_COUNT_ADDR as u32);
    load_isr(bus);
}

/// Firmware C — mid-run reconfiguration: arm PSC=1/ARR=30/UIE, poll CNT for
/// 200 loop passes (several update IRQs land), then rewrite PSC=5, ARR=20 and
/// fire a software update (EGR.UG); spin polling after.
///
///   0x40: 480B  ldr r0, [pc, #44]   ; = NVIC_ISER0 (pool at 0x70)
///   0x42: 490C  ldr r1, [pc, #48]   ; = 1 << 28    (pool at 0x74)
///   0x44: 6001  str r1, [r0]
///   0x46: 480C  ldr r0, [pc, #48]   ; = TIM_BASE   (pool at 0x78)
///   0x48: 2101  movs r1, #1
///   0x4A: 6281  str r1, [r0, #0x28] ; PSC = 1
///   0x4C: 211E  movs r1, #30
///   0x4E: 62C1  str r1, [r0, #0x2C] ; ARR = 30
///   0x50: 2101  movs r1, #1
///   0x52: 60C1  str r1, [r0, #0x0C] ; DIER = UIE
///   0x54: 6001  str r1, [r0, #0x00] ; CR1 = CEN
///   0x56: 25C8  movs r5, #200
///   d1:
///   0x58: 6A44  ldr r4, [r0, #0x24] ; poll CNT
///   0x5A: 3D01  subs r5, #1
///   0x5C: D1FC  bne d1
///   0x5E: 2105  movs r1, #5
///   0x60: 6281  str r1, [r0, #0x28] ; PSC = 5 (immediate-apply, mid-phase)
///   0x62: 2114  movs r1, #20
///   0x64: 62C1  str r1, [r0, #0x2C] ; ARR = 20
///   0x66: 2101  movs r1, #1
///   0x68: 6141  str r1, [r0, #0x14] ; EGR.UG (software update)
///   loop:
///   0x6A: 3301  adds r3, #1
///   0x6C: 6A44  ldr r4, [r0, #0x24]
///   0x6E: E7FC  b loop
fn load_firmware_reconfig(bus: &mut SystemBus) {
    load_thumb(
        bus,
        0x40,
        &[
            0x480B, 0x490C, 0x6001, 0x480C, 0x2101, 0x6281, 0x211E, 0x62C1, 0x2101, 0x60C1, 0x6001,
            0x25C8, 0x6A44, 0x3D01, 0xD1FC, 0x2105, 0x6281, 0x2114, 0x62C1, 0x2101, 0x6141, 0x3301,
            0x6A44, 0xE7FC,
        ],
    );
    write_word(bus, 0x70, NVIC_ISER0);
    write_word(bus, 0x74, 1 << TIM_IRQ);
    write_word(bus, 0x78, TIM_BASE);
    load_isr(bus);
}

/// Firmware D — the DIER=0 polling shape: PSC=3, ARR=9, no interrupts; every
/// loop pass stores raw CNT and SR to RAM (the lazily-latched-flag surface).
///
///   0x40: 4807  ldr r0, [pc, #28]   ; = TIM_BASE (pool at 0x60)
///   0x42: 2103  movs r1, #3
///   0x44: 6281  str r1, [r0, #0x28] ; PSC = 3
///   0x46: 2109  movs r1, #9
///   0x48: 62C1  str r1, [r0, #0x2C] ; ARR = 9
///   0x4A: 2101  movs r1, #1
///   0x4C: 6001  str r1, [r0, #0x00] ; CR1 = CEN
///   0x4E: 4A05  ldr r2, [pc, #20]   ; = 0x2000_0008 (pool at 0x64)
///   loop:
///   0x50: 6A44  ldr r4, [r0, #0x24] ; CNT
///   0x52: 6014  str r4, [r2]
///   0x54: 6905  ldr r5, [r0, #0x10] ; SR (lazy latch surface)
///   0x56: 6055  str r5, [r2, #4]
///   0x58: E7FA  b loop
///   0x5A: BF00  nop
///   0x5C: BF00  nop (pool align)
fn load_firmware_poll(bus: &mut SystemBus) {
    load_thumb(
        bus,
        0x40,
        &[
            0x4807, 0x2103, 0x6281, 0x2109, 0x62C1, 0x2101, 0x6001, 0x4A05, 0x6A44, 0x6014, 0x6905,
            0x6055, 0xE7FA, 0xBF00, 0xBF00,
        ],
    );
    write_word(bus, 0x60, TIM_BASE);
    write_word(bus, 0x64, 0x2000_0008);
}

/// Firmware E — the interval-64 count gate: DIER=0, PSC=3, ARR=99 (update
/// every 400 cycles); the main loop polls SR, and on UIF clears SR and
/// increments the wrap counter at `MAIN_COUNT_ADDR`. The latched flag cannot
/// be missed at any tick interval, so the wrap count is exact.
///
///   0x40: 4809  ldr r0, [pc, #36]   ; = TIM_BASE (pool at 0x68)
///   0x42: 2103  movs r1, #3
///   0x44: 6281  str r1, [r0, #0x28] ; PSC = 3
///   0x46: 2163  movs r1, #99
///   0x48: 62C1  str r1, [r0, #0x2C] ; ARR = 99
///   0x4A: 2101  movs r1, #1
///   0x4C: 6001  str r1, [r0, #0x00] ; CR1 = CEN (DIER stays 0)
///   0x4E: 4A07  ldr r2, [pc, #28]   ; = MAIN_COUNT_ADDR (pool at 0x6C)
///   loop:
///   0x50: 6905  ldr r5, [r0, #0x10] ; SR (lazy latch surface)
///   0x52: 07ED  lsls r5, r5, #31    ; UIF → bit 31, sets Z
///   0x54: D004  beq skip
///   0x56: 2100  movs r1, #0
///   0x58: 6101  str r1, [r0, #0x10] ; clear SR
///   0x5A: 6813  ldr r3, [r2]
///   0x5C: 3301  adds r3, #1
///   0x5E: 6013  str r3, [r2]        ; wrap_count++
///   skip:
///   0x60: E7F6  b loop
///   0x62: BF00  nop
///   0x64: BF00  nop (pool align)
fn load_firmware_wrap_poll(bus: &mut SystemBus) {
    load_thumb(
        bus,
        0x40,
        &[
            0x4809, 0x2103, 0x6281, 0x2163, 0x62C1, 0x2101, 0x6001, 0x4A07, 0x6905, 0x07ED, 0xD004,
            0x2100, 0x6101, 0x6813, 0x3301, 0x6013, 0xE7F6, 0xBF00, 0xBF00,
        ],
    );
    write_word(bus, 0x68, TIM_BASE);
    write_word(bus, 0x6C, MAIN_COUNT_ADDR as u32);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    step: u64,
    total_cycles: u64,
    pc: u32,
    regs: [u32; 16],
    isr_count: u32,
    main_count: u32,
    poll_out: [u32; 2],
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
        isr_count: machine.bus.read_u32(ISR_COUNT_ADDR).unwrap(),
        main_count: machine.bus.read_u32(MAIN_COUNT_ADDR).unwrap(),
        poll_out: [
            machine.bus.read_u32(0x2000_0008).unwrap(),
            machine.bus.read_u32(0x2000_000C).unwrap(),
        ],
    }
}

/// Run `steps` instructions one at a time through the batched `Machine::run`
/// path (the production path the scheduler timing convention is calibrated
/// to), probing the full architectural state after every instruction.
fn run_probed(machine: &mut Machine<CortexM>, entry: u32, steps: u64) -> Vec<Probe> {
    machine.cpu.pc = entry;
    let mut probes = Vec::with_capacity(steps as usize);
    for s in 0..steps {
        machine.run(Some(1)).unwrap();
        probes.push(probe(machine, s + 1));
    }
    probes
}

fn assert_probes_identical(reference: &[Probe], candidate: &[Probe], what: &str) {
    assert_eq!(reference.len(), candidate.len());
    for (r, c) in reference.iter().zip(candidate.iter()) {
        assert_eq!(
            r, c,
            "{what}: first divergence at step {} (walk-reference vs scheduler)",
            r.step
        );
    }
}

fn run_differential(load: fn(&mut SystemBus), steps: u64, what: &str) -> (Vec<Probe>, Vec<Probe>) {
    let mut walk = build_machine(false, 1);
    load(&mut walk.bus);
    let walk_probes = run_probed(&mut walk, 0x40, steps);

    let mut sched = build_machine(true, 1);
    load(&mut sched.bus);
    let sched_probes = run_probed(&mut sched, 0x40, steps);

    assert_probes_identical(&walk_probes, &sched_probes, what);
    (walk_probes, sched_probes)
}

/// Gate 1: update-event (UIF/UIE) NVIC IRQ + lazy CNT polling — walk-on vs
/// scheduler at tick interval 1, every instruction-boundary observable
/// byte-identical (IRQ delivery cycles including the level-repend artifact,
/// held-level counter freeze, ISR execution count, total_cycles, registers —
/// r4 carries the raw CNT poll).
#[test]
fn update_irq_firmware_is_byte_identical_at_interval_1() {
    const STEPS: u64 = 4_000;
    let (walk_probes, _) =
        run_differential(load_firmware_update_irq, STEPS, "timer update-IRQ firmware");
    let last = walk_probes.last().unwrap();
    assert!(
        last.isr_count >= 15,
        "reference must take repeated update ISRs (got {})",
        last.isr_count
    );
    assert!(last.main_count > 300, "main loop must run");
}

/// Gate 2: compare-match (CC1IF/CC1IE) — the latch lands on the match tick,
/// the level pend one tick later; byte-identity pins both.
#[test]
fn compare_match_irq_firmware_is_byte_identical_at_interval_1() {
    const STEPS: u64 = 4_000;
    let (walk_probes, _) = run_differential(
        load_firmware_compare_irq,
        STEPS,
        "timer compare-IRQ firmware",
    );
    let last = walk_probes.last().unwrap();
    assert!(
        last.isr_count >= 15,
        "reference must take repeated compare ISRs (got {})",
        last.isr_count
    );
}

/// Gate 3: mid-run PSC/ARR rewrite + EGR.UG — reconfiguration must cancel and
/// re-arm the event chain at the exact walk cycles (immediate-apply PSC
/// phase, the model's pinned no-buffered-reload semantics).
#[test]
fn mid_run_reconfig_firmware_is_byte_identical_at_interval_1() {
    const STEPS: u64 = 4_000;
    let (walk_probes, _) = run_differential(
        load_firmware_reconfig,
        STEPS,
        "timer mid-run-reconfig firmware",
    );
    let last = walk_probes.last().unwrap();
    assert!(
        last.isr_count >= 5,
        "reference must take ISRs across the reconfiguration (got {})",
        last.isr_count
    );
}

/// Gate 4: DIER=0 polling — lazy CNT and lazily-latched SR flag reads with no
/// freeze and no IRQs, byte-identical every instruction.
#[test]
fn cnt_sr_poll_firmware_is_byte_identical_at_interval_1() {
    const STEPS: u64 = 2_000;
    let (walk_probes, _) =
        run_differential(load_firmware_poll, STEPS, "timer CNT/SR poll firmware");
    let last = walk_probes.last().unwrap();
    assert!(
        last.poll_out[1] & 1 == 1 || walk_probes.iter().any(|p| p.poll_out[1] & 1 == 1),
        "the poll loop must observe a lazily-latched UIF"
    );
}

/// Gate 5: scheduler @ interval 64 vs the walk-on interval-1 golden
/// reference, on the DIER=0 UIF-poll-and-clear shape. Lazy reads quantise to
/// the batch grid (≤ one interval, documented), so per-instruction state is
/// NOT compared — but the latched UIF can never be missed, so the observed
/// update-event count over the fixed window must be EXACT (absolute
/// closed-form deadlines — no cumulative drift), with the window edge
/// verified to be more than one interval + poll-loop length away from any
/// wrap observation in the reference.
#[test]
fn wrap_count_is_exact_at_interval_64() {
    const STEPS: u64 = 6_000;

    let mut walk = build_machine(false, 1);
    load_firmware_wrap_poll(&mut walk.bus);
    let walk_probes = run_probed(&mut walk, 0x40, STEPS);

    let mut sched = build_machine(true, 64);
    load_firmware_wrap_poll(&mut sched.bus);
    sched.cpu.pc = 0x40;
    sched.run(Some(STEPS as u32)).unwrap();

    let reference = walk_probes.last().unwrap();
    let sched_count = sched.bus.read_u32(MAIN_COUNT_ADDR).unwrap();

    // Quantisation can only DELAY an observation (lazy reads trail by < one
    // interval; the poll loop adds a bounded lag). The count can only differ
    // if the last wrap observation sits within ~(64 + loop length) cycles of
    // the window edge — verify the fixture stays clear (wrap period is 400).
    let last_observation_step = walk_probes
        .iter()
        .zip(walk_probes.iter().skip(1))
        .filter(|(a, b)| b.main_count > a.main_count)
        .map(|(_, b)| b.step)
        .next_back()
        .expect("reference observes at least one wrap");
    assert!(
        STEPS - last_observation_step > 100,
        "fixture must keep the window edge > interval + poll lag from the last \
         wrap observation (last at step {last_observation_step})"
    );

    assert!(
        reference.main_count >= 10,
        "reference must observe repeated update events (got {})",
        reference.main_count
    );
    assert_eq!(
        sched_count, reference.main_count,
        "update-event count over the fixed window must be exact at interval 64"
    );
    // total_cycles advances one per instruction in both lanes (B2/B3 removed
    // the legacy timer tick-cost artifact), so the window length matches.
    assert_eq!(sched.total_cycles, reference.total_cycles);
}
