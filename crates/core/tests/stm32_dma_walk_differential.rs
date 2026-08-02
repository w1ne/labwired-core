// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential gate for walk-free batch **B4** (STM32 DMA1/DMA2 → event
//! scheduler), in the `stm32_timer_walk_differential` (B2/B3) style: the SAME
//! hand-built Cortex-M machine and hand-assembled Thumb firmware run twice —
//! once with the DMA pinned back onto the per-cycle walk (`force_legacy_walk`,
//! the reference) and once scheduler-driven — and every observable is compared
//! after EVERY instruction.
//!
//! 1. `mem2mem_memcpy_tcie_firmware_is_byte_identical_at_interval_1` — firmware
//!    programs a DMA1 channel-1 memory-to-memory transfer (CPAR=dst, CMAR=src,
//!    CNDTR=N, CCR=EN|MEM2MEM|MINC|PINC|TCIE), enables the channel's NVIC line,
//!    and spins polling ISR while a main-loop counter runs; the transfer-
//!    complete interrupt (TCIF) preempts into an ISR that clears the flag and
//!    counts. Probed after every instruction: total_cycles, PC, all 16 core
//!    registers, the destination buffer, and the ISR/main counters must be
//!    byte-identical at tick interval 1 — pinning the per-element transfer
//!    cycles, the TCIF/GIF latch cycle, the NVIC IRQ delivery cycle, and the
//!    copied destination bytes exactly.
//!
//! 2. `mem2mem_memcpy_is_byte_identical_at_interval_64` — the SAME memcpy
//!    firmware run walk-on vs scheduler with BOTH lanes at tick interval 64,
//!    compared per instruction. Unlike the timer's absolute-deadline update
//!    events, a mem2mem transfer is a *relative* delay-1 element chain (one
//!    element per bus tick), so at interval N it paces N× slower — the SAME
//!    behaviour as the legacy walk at interval N (which also services one
//!    element per `tick_elapsed`). The gate therefore pins the scheduler to the
//!    walk at the batched interval directly: DMA state (CNDTR, ISR flags, the
//!    copied bytes) advances only at the shared 64-cycle tick/drain boundary in
//!    both lanes, and the TCIF/NVIC delivery lands on the same boundary, so the
//!    per-instruction trace is byte-identical.

#![cfg(feature = "event-scheduler")]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::peripherals::dma::Dma1;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};

const DMA_BASE: u32 = 0x4002_0000;
/// Hand-built NVIC position for the DMA entry — exception number 16 + 28 = 44,
/// vector at 44*4 = 0xB0 (above the firmware + ISR + literal pools, matching the
/// timer differential's layout). The number is arbitrary: `add_peripheral`
/// assigns whatever line we pass, so this pins IRQ *delivery timing*, not the
/// silicon DMA1_Channel1 line number.
const DMA_IRQ: u32 = 28;
const NVIC_ISER0: u32 = 0xE000_E100;

const SRC_ADDR: u32 = 0x2000_0100;
const DST_ADDR: u32 = 0x2000_0200;
const ISR_COUNT_ADDR: u64 = 0x2000_0000;
const MAIN_COUNT_ADDR: u64 = 0x2000_0004;
const INITIAL_SP: u32 = 0x2000_8000;
/// Transfer length (elements == bytes; byte width, PINC/MINC stride 1).
const N: u32 = 16;

fn load_thumb(bus: &mut SystemBus, base: u64, halfwords: &[u16]) {
    for (i, hw) in halfwords.iter().enumerate() {
        bus.write_u16(base + (i as u64) * 2, *hw).unwrap();
    }
}

fn write_word(bus: &mut SystemBus, addr: u64, word: u32) {
    bus.write_u32(addr, word).unwrap();
}

/// Fill the DMA source buffer with a recognisable pattern so the destination
/// bytes are a meaningful equality check, not all-zero.
fn fill_source(bus: &mut SystemBus) {
    for i in 0..N {
        bus.write_u8(SRC_ADDR as u64 + i as u64, 0xA0 ^ (i as u8).wrapping_mul(7))
            .unwrap();
    }
}

/// Build the shared machine assembly: `SystemBus::new()` + `configure_cortex_m`
/// (real SCB/NVIC/DWT) + a native `Dma1` registered at `DMA_BASE` with NVIC
/// line 28 through `add_peripheral` — which attaches the bus cycle clock exactly
/// as the production `from_config` choke would. `scheduler = false` then pins
/// the DMA back onto the legacy walk (`force_legacy_walk`) to form the reference
/// lane from the same assembly.
fn build_machine(scheduler: bool, tick_interval: u32) -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);

    bus.add_peripheral(
        "dma1",
        DMA_BASE as u64,
        0x400,
        Some(DMA_IRQ),
        Box::new(Dma1::new()),
    );

    if !scheduler {
        let idx = bus.find_peripheral_index_by_name("dma1").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Dma1>()
            .unwrap()
            .force_legacy_walk();
    }

    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    machine.cpu.sp = INITIAL_SP;
    machine
}

/// The shared ISR at 0x80: clear DMA CH1 flags (GIF|TCIF|HTIF via IFCR), then
/// increment the word at `ISR_COUNT_ADDR` and return.
///
///   0x80: 4803  ldr r0, [pc, #12]   ; = DMA_BASE (pool at 0x90)
///   0x82: 2107  movs r1, #7
///   0x84: 6041  str r1, [r0, #0x04] ; IFCR = 0x7 (clear CH1 GIF|TCIF|HTIF)
///   0x86: 4803  ldr r0, [pc, #12]   ; = ISR_COUNT_ADDR (pool at 0x94)
///   0x88: 6801  ldr r1, [r0]
///   0x8A: 3101  adds r1, #1
///   0x8C: 6001  str r1, [r0]
///   0x8E: 4770  bx lr
///   0x90: .word DMA_BASE
///   0x94: .word ISR_COUNT_ADDR
const ISR_BASE: u64 = 0x80;
fn load_isr(bus: &mut SystemBus) {
    load_thumb(
        bus,
        ISR_BASE,
        &[
            0x4803, 0x2107, 0x6041, 0x4803, 0x6801, 0x3101, 0x6001, 0x4770,
        ],
    );
    write_word(bus, ISR_BASE + 0x10, DMA_BASE);
    write_word(bus, ISR_BASE + 0x14, ISR_COUNT_ADDR as u32);
    // Vector for exception 16 + DMA_IRQ = 44 → offset 0xB0.
    write_word(bus, (16 + DMA_IRQ) as u64 * 4, (ISR_BASE as u32) | 1);
}

/// Firmware A — one-shot mem2mem memcpy with TCIE:
///
///   0x40: 4808  ldr r0, [pc, #32]   ; = NVIC_ISER0     (pool at 0x64)
///   0x42: 4909  ldr r1, [pc, #36]   ; = 1 << DMA_IRQ   (pool at 0x68)
///   0x44: 6001  str r1, [r0]        ; NVIC enable
///   0x46: 4809  ldr r0, [pc, #36]   ; = DMA_BASE       (pool at 0x6C)
///   0x48: 4909  ldr r1, [pc, #36]   ; = DST_ADDR       (pool at 0x70)
///   0x4A: 6101  str r1, [r0, #0x10] ; CPAR = dst
///   0x4C: 4909  ldr r1, [pc, #36]   ; = SRC_ADDR       (pool at 0x74)
///   0x4E: 6141  str r1, [r0, #0x14] ; CMAR = src
///   0x50: 2110  movs r1, #N (=16)
///   0x52: 60C1  str r1, [r0, #0x0C] ; CNDTR = N
///   0x54: 4908  ldr r1, [pc, #32]   ; = CCR value      (pool at 0x78)
///   0x56: 6081  str r1, [r0, #0x08] ; CCR = EN|MEM2MEM|DIR|MINC|PINC|TCIE
///   0x58: 4A08  ldr r2, [pc, #32]   ; = MAIN_COUNT_ADDR(pool at 0x7C)
///   0x5A: 2300  movs r3, #0
///   loop:
///   0x5C: 3301  adds r3, #1
///   0x5E: 6013  str r3, [r2]
///   0x60: 6804  ldr r4, [r0]        ; poll ISR
///   0x62: E7FB  b loop
/// CCR = EN|TCIE|DIR|PINC|MINC|MEM2MEM = bits 0,1,4,6,7,14 = 0x40D3.
fn load_firmware_memcpy(bus: &mut SystemBus) {
    load_thumb(
        bus,
        0x40,
        &[
            0x4808,
            0x4909,
            0x6001,
            0x4809,
            0x4909,
            0x6101,
            0x4909,
            0x6141,
            0x2100 | (N as u16),
            0x60C1,
            0x4908,
            0x6081,
            0x4A08,
            0x2300,
            0x3301,
            0x6013,
            0x6804,
            0xE7FB,
        ],
    );
    write_word(bus, 0x64, NVIC_ISER0);
    write_word(bus, 0x68, 1 << DMA_IRQ);
    write_word(bus, 0x6C, DMA_BASE);
    write_word(bus, 0x70, DST_ADDR);
    write_word(bus, 0x74, SRC_ADDR);
    write_word(
        bus,
        0x78,
        (1 << 0) | (1 << 1) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 14),
    );
    write_word(bus, 0x7C, MAIN_COUNT_ADDR as u32);
    load_isr(bus);
    fill_source(bus);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    step: u64,
    total_cycles: u64,
    pc: u32,
    regs: [u32; 16],
    isr_count: u32,
    main_count: u32,
    dst: Vec<u8>,
}

fn probe(machine: &Machine<CortexM>, step: u64) -> Probe {
    let mut regs = [0u32; 16];
    for (i, r) in regs.iter_mut().enumerate() {
        *r = machine.read_core_reg(i as u8);
    }
    let dst = (0..N)
        .map(|i| machine.bus.read_u8(DST_ADDR as u64 + i as u64).unwrap())
        .collect();
    Probe {
        step,
        total_cycles: machine.total_cycles,
        pc: machine.get_pc(),
        regs,
        isr_count: machine.bus.read_u32(ISR_COUNT_ADDR).unwrap(),
        main_count: machine.bus.read_u32(MAIN_COUNT_ADDR).unwrap(),
        dst,
    }
}

fn run_probed(machine: &mut Machine<CortexM>, entry: u32, steps: u64) -> Vec<Probe> {
    machine.cpu.pc = entry;
    let mut probes = Vec::with_capacity(steps as usize);
    for s in 0..steps {
        machine.run(Some(1)).unwrap();
        probes.push(probe(machine, s + 1));
    }
    probes
}

fn run_differential(load: fn(&mut SystemBus), steps: u64, interval: u32, what: &str) -> Vec<Probe> {
    let mut walk = build_machine(false, interval);
    load(&mut walk.bus);
    let walk_probes = run_probed(&mut walk, 0x40, steps);

    let mut sched = build_machine(true, interval);
    load(&mut sched.bus);
    let sched_probes = run_probed(&mut sched, 0x40, steps);

    assert_eq!(walk_probes.len(), sched_probes.len());
    for (r, c) in walk_probes.iter().zip(sched_probes.iter()) {
        assert_eq!(
            r, c,
            "{what}: first divergence at step {} (walk-reference vs scheduler)",
            r.step
        );
    }
    walk_probes
}

/// Gate 1: mem2mem memcpy + TCIE NVIC IRQ — walk-on vs scheduler at tick
/// interval 1, every instruction-boundary observable byte-identical (per-element
/// transfer cycles, the copied destination bytes, TCIF latch + NVIC delivery
/// cycle, ISR execution count, total_cycles, registers — r4 carries the ISR
/// poll).
#[test]
fn mem2mem_memcpy_tcie_firmware_is_byte_identical_at_interval_1() {
    const STEPS: u64 = 800;
    let walk_probes = run_differential(
        load_firmware_memcpy,
        STEPS,
        1,
        "DMA mem2mem memcpy firmware (interval 1)",
    );
    let last = walk_probes.last().unwrap();
    assert_eq!(last.isr_count, 1, "reference must take exactly one TC ISR");
    assert!(last.main_count > 100, "main loop must run");
    // The destination must hold the source pattern after the transfer.
    let expected: Vec<u8> = (0..N).map(|i| 0xA0u8 ^ (i as u8).wrapping_mul(7)).collect();
    assert_eq!(last.dst, expected, "mem2mem must copy the source bytes");
}

/// Gate 2: the SAME memcpy firmware, walk-on vs scheduler, BOTH lanes at tick
/// interval 64, run in ONE batched `run(STEPS)` call (the production batched
/// path, NOT single-stepped — single-stepping would force batch=1 and drain the
/// scheduler every cycle while the walk tick only fires on 64-boundaries,
/// quantising the two lanes differently). A mem2mem transfer paces one element
/// per bus tick in both lanes, so at interval 64 the transfer, the TCIF/GIF
/// latch, the copied bytes, and the NVIC delivery all advance on the shared
/// 64-cycle boundary — the FINAL architectural + memory state is identical.
#[test]
fn mem2mem_memcpy_is_byte_identical_at_interval_64() {
    // A 16-element transfer paces one element per 64-cycle tick at interval 64,
    // so completion + the TC ISR need > 16*64 ≈ 1024 cycles; 4000 steps gives
    // margin for the ISR and the trailing poll loop.
    const STEPS: u64 = 4_000;

    let mut walk = build_machine(false, 64);
    load_firmware_memcpy(&mut walk.bus);
    walk.cpu.pc = 0x40;
    walk.run(Some(STEPS as u32)).unwrap();
    let walk_probe = probe(&walk, STEPS);

    let mut sched = build_machine(true, 64);
    load_firmware_memcpy(&mut sched.bus);
    sched.cpu.pc = 0x40;
    sched.run(Some(STEPS as u32)).unwrap();
    let sched_probe = probe(&sched, STEPS);

    assert_eq!(
        walk_probe, sched_probe,
        "interval-64 batched run: final state diverged (walk vs scheduler)"
    );
    assert_eq!(
        walk_probe.isr_count, 1,
        "the TC ISR must fire once at interval 64"
    );
    let expected: Vec<u8> = (0..N).map(|i| 0xA0u8 ^ (i as u8).wrapping_mul(7)).collect();
    assert_eq!(
        walk_probe.dst, expected,
        "mem2mem copies the source bytes at interval 64"
    );
}
