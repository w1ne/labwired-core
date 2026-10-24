// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32H563 (Cortex-M33) executing walk-vs-scheduler fidelity differentials.
//!
//! ## 1. Zephyr boot (SysTick / console)
//!
//! Closes the H563 gap on the core-timer path: stock Zephyr `hello_world` for
//! `nucleo_h563zi` on the real `stm32h563` `from_config` bus, once with
//! SysTick/SCB/DWT + SoC timing models pinned to the legacy walk and once
//! scheduler-driven — boot trace + console stream byte-identical.
//! See `stm32_walk_free`.
//!
//! ## 2. GPDMA1 mem-to-mem transfer completion (hand-built)
//!
//! Same shape as `stm32_dma_walk_differential` (B4): Cortex-M + native `Gpdma`
//! at the silicon GPDMA1 base, SWREQ byte mem-to-mem with TCIE. Walk lane uses
//! `force_legacy_walk`; scheduler lane rides the delay-0/1 element chain.
//!
//! * Interval 1: every instruction-boundary probe is byte-identical (copied
//!   destination bytes, CSR TCF/HTF, NVIC TC ISR count, registers).
//! * Interval 512: both lanes at 512, one batched `run` — final architectural
//!   + memory state identical. Relative delay-1 pacing is N× slower at interval
//!     N in *both* lanes (same as classic DMA mem2mem).
//!
//! ## 3. RTC v3 second boundary + alarm compare (hand-built)
//!
//! Same shape as `stm32_timer_walk_differential` absolute-deadline gates: native
//! `RtcV3` with calendar second divider [`TICKS_PER_SECOND`] and Alarm A match.
//! Host programs WPR/init/TR/ALRMAR/CR; firmware polls TR + SR.
//!
//! * Interval 1: per-instruction byte-identical (TR advance + SR.ALRAF latch).
//! * Interval 512: scheduler vs walk@1 golden — observed second / alarm count
//!   over a fixed instruction window is EXACT (absolute closed-form second
//!   deadlines; no cumulative drift), provided the window edge sits clear of
//!   a second boundary (asserted from the reference trace).

#![cfg(feature = "event-scheduler")]

#[path = "stm32_walk_free/mod.rs"]
mod harness;

mod common;

use common::walk_differential::{assert_walk_differential, probe_at, run_probed, WalkMode};
use labwired_core::bus::{SystemBus, RECOMMENDED_TICK_INTERVAL};
use labwired_core::cpu::CortexM;
use labwired_core::peripherals::gpdma::Gpdma;
use labwired_core::peripherals::rtc_v3::{RtcV3, TICKS_PER_SECOND};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, DebugControl, Machine};

// ── Shared Thumb helpers ────────────────────────────────────────────────────

const INITIAL_SP: u32 = 0x2000_8000;
const ISR_COUNT_ADDR: u64 = 0x2000_0000;
const MAIN_COUNT_ADDR: u64 = 0x2000_0004;
const NVIC_ISER0: u32 = 0xE000_E100;

fn load_thumb(bus: &mut SystemBus, base: u64, halfwords: &[u16]) {
    for (i, hw) in halfwords.iter().enumerate() {
        bus.write_u16(base + (i as u64) * 2, *hw).unwrap();
    }
}

fn write_word(bus: &mut SystemBus, addr: u64, word: u32) {
    bus.write_u32(addr, word).unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// Zephyr boot
// ═══════════════════════════════════════════════════════════════════════════

/// H563 kernel-tick (SysTick) IRQ-cadence + console differential over the real
/// Zephyr boot.
#[test]
fn stm32h563_zephyr_boot_walk_vs_scheduler_is_byte_identical() {
    harness::assert_walk_free_boot_identical(
        "stm32h563",
        "nucleo-h563zi-demo",
        "stm32h563-zephyr-hello.elf",
        b"Hello World! nucleo_h563zi",
        800_000,
        50_000,
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// GPDMA1 mem-to-mem
// ═══════════════════════════════════════════════════════════════════════════

/// Silicon GPDMA1 base (RM0481); ch0 NVIC line 27 on H563 (stm32h563xx.h).
/// Hand-built IRQ 28 keeps the vector at 0xB0 (exception 44), matching the
/// classic DMA differential layout.
const GPDMA_BASE: u32 = 0x4002_0000;
const GPDMA_IRQ: u32 = 28;
const GPDMA_SIZE: u64 = 0x1000;

const CH0_CFCR: u32 = GPDMA_BASE + 0x5C;
const CH0_CSR: u32 = GPDMA_BASE + 0x60;
const CH0_CCR: u32 = GPDMA_BASE + 0x64;
const CH0_CTR1: u32 = GPDMA_BASE + 0x90;
const CH0_CTR2: u32 = GPDMA_BASE + 0x94;
const CH0_CBR1: u32 = GPDMA_BASE + 0x98;
const CH0_CSAR: u32 = GPDMA_BASE + 0x9C;
const CH0_CDAR: u32 = GPDMA_BASE + 0xA0;

const CTR1_SINC_DINC: u32 = (1 << 3) | (1 << 19);
const CTR2_SWREQ: u32 = 1 << 9;
const CCR_EN_TCIE: u32 = (1 << 0) | (1 << 8);

const GPDMA_SRC: u32 = 0x2000_0100;
const GPDMA_DST: u32 = 0x2000_0200;
const GPDMA_N: u32 = 16;

const GPDMA_FW_BASE: u64 = 0x40;
const GPDMA_ISR_BASE: u64 = 0x80;

fn build_gpdma_machine(scheduler: bool, tick_interval: u32) -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);

    bus.add_peripheral(
        "gpdma1",
        GPDMA_BASE as u64,
        GPDMA_SIZE,
        Some(GPDMA_IRQ),
        Box::new(Gpdma::new().with_base(GPDMA_BASE)),
    );

    if !scheduler {
        let idx = bus.find_peripheral_index_by_name("gpdma1").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Gpdma>()
            .unwrap()
            .force_legacy_walk();
    }

    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    machine.cpu.sp = INITIAL_SP;
    machine
}

fn fill_gpdma_source(bus: &mut SystemBus) {
    for i in 0..GPDMA_N {
        bus.write_u8(
            GPDMA_SRC as u64 + i as u64,
            0xA0u8 ^ (i as u8).wrapping_mul(7),
        )
        .unwrap();
        bus.write_u8(GPDMA_DST as u64 + i as u64, 0).unwrap();
    }
}

/// ISR @ 0x80: CFCR w1c (clear TCF|HTF), isr_count++, bx lr.
///
///   0x80: 4803  ldr r0, [pc, #12]   ; = CH0_CFCR        @ 0x90
///   0x82: 4904  ldr r1, [pc, #16]   ; = 0x300 (TCF|HTF) @ 0x94
///   0x84: 6001  str r1, [r0]
///   0x86: 4804  ldr r0, [pc, #16]   ; = ISR_COUNT       @ 0x98
///   0x88: 6801  ldr r1, [r0]
///   0x8A: 3101  adds r1, #1
///   0x8C: 6001  str r1, [r0]
///   0x8E: 4770  bx lr
fn load_gpdma_isr(bus: &mut SystemBus) {
    // 0x80: Align(0x84)=0x84 + 12 = 0x90
    // 0x82: Align(0x86)=0x84 + 16 = 0x94
    // 0x86: Align(0x8A)=0x88 + 16 = 0x98
    load_thumb(
        bus,
        GPDMA_ISR_BASE,
        &[
            0x4803, 0x4904, 0x6001, 0x4804, 0x6801, 0x3101, 0x6001, 0x4770,
        ],
    );
    write_word(bus, 0x90, CH0_CFCR);
    write_word(bus, 0x94, 0x0000_0300);
    write_word(bus, 0x98, ISR_COUNT_ADDR as u32);
    write_word(
        bus,
        (16 + GPDMA_IRQ) as u64 * 4,
        (GPDMA_ISR_BASE as u32) | 1,
    );
}

/// Host programs CTR1/CTR2/CBR1/CSAR/CDAR (transfer not yet enabled). Firmware
/// enables the NVIC line, writes CCR=EN|TCIE to start, then polls CSR.
///
/// Keeping programming on the host avoids a large Thumb literal pool; the
/// fidelity surface under test is the **transfer cadence + TC IRQ**, which
/// arms on the CCR.EN write (identical MMIO harvest path in both lanes).
fn program_gpdma_channel(bus: &mut SystemBus) {
    bus.write_u32(CH0_CTR1 as u64, CTR1_SINC_DINC).unwrap();
    bus.write_u32(CH0_CTR2 as u64, CTR2_SWREQ).unwrap();
    bus.write_u32(CH0_CBR1 as u64, GPDMA_N).unwrap();
    bus.write_u32(CH0_CSAR as u64, GPDMA_SRC).unwrap();
    bus.write_u32(CH0_CDAR as u64, GPDMA_DST).unwrap();
}

///   0x40: 4808 → 0x64  NVIC_ISER0
///   0x42: 4909 → 0x68  1<<IRQ
///   0x44: 6001
///   0x46: 4809 → 0x6C  CH0_CCR
///   0x48: 4909 → 0x70  EN|TCIE
///   0x4A: 6001              ; start transfer
///   0x4C: 4809 → 0x74  CH0_CSR
///   0x4E: 4A0A → 0x78  MAIN_COUNT
///   0x50: 2300  movs r3, #0
/// loop @ 0x52:
///   0x52: 3301  adds r3, #1
///   0x54: 6013  str r3, [r2]
///   0x56: 6804  ldr r4, [r0]
///   0x58: E7FB  b 0x52
fn load_gpdma_memcpy(bus: &mut SystemBus) {
    load_thumb(
        bus,
        GPDMA_FW_BASE,
        &[
            0x4808, 0x4909, 0x6001, 0x4809, 0x4909, 0x6001, 0x4809, 0x4A0A, 0x2300, 0x3301, 0x6013,
            0x6804, 0xE7FB,
        ],
    );
    write_word(bus, 0x64, NVIC_ISER0);
    write_word(bus, 0x68, 1 << GPDMA_IRQ);
    write_word(bus, 0x6C, CH0_CCR);
    write_word(bus, 0x70, CCR_EN_TCIE);
    write_word(bus, 0x74, CH0_CSR);
    write_word(bus, 0x78, MAIN_COUNT_ADDR as u32);

    load_gpdma_isr(bus);
    fill_gpdma_source(bus);
    program_gpdma_channel(bus);
}

/// The GPDMA destination buffer as two little-endian u64 words — byte-equality
/// preserved exactly, with the failure text naming the differing half.
fn gpdma_dst_words(machine: &Machine<CortexM>) -> (u64, u64) {
    let word = |off: u64| {
        let mut w = [0u8; 8];
        for (i, b) in w.iter_mut().enumerate() {
            *b = machine
                .bus
                .read_u8(GPDMA_DST as u64 + off + i as u64)
                .unwrap();
        }
        u64::from_le_bytes(w)
    };
    (word(0), word(8))
}

/// The caller-supplied observables the harness probe carries for the GPDMA
/// fixture: the two RAM counters, the channel CSR and the destination words.
fn extra_gpdma(machine: &Machine<CortexM>) -> Vec<(&'static str, u64)> {
    let (lo, hi) = gpdma_dst_words(machine);
    vec![
        (
            "isr_count",
            u64::from(machine.bus.read_u32(ISR_COUNT_ADDR).unwrap()),
        ),
        (
            "main_count",
            u64::from(machine.bus.read_u32(MAIN_COUNT_ADDR).unwrap()),
        ),
        (
            "csr",
            u64::from(machine.bus.read_u32(CH0_CSR as u64).unwrap()),
        ),
        ("dst_lo", lo),
        ("dst_hi", hi),
    ]
}

fn expected_gpdma_dst() -> (u64, u64) {
    let bytes: Vec<u8> = (0..GPDMA_N)
        .map(|i| 0xA0u8 ^ (i as u8).wrapping_mul(7))
        .collect();
    let lo = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
    let hi = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    (lo, hi)
}

/// Gate: GPDMA mem2mem + TCIE — walk-on vs scheduler at tick interval 1,
/// every instruction-boundary observable byte-identical.
#[test]
fn gpdma_mem2mem_tcie_is_byte_identical_at_interval_1() {
    const STEPS: u64 = 800;

    let build = |mode: WalkMode| {
        let mut m = build_gpdma_machine(!mode.is_legacy_walk(), 1);
        load_gpdma_memcpy(&mut m.bus);
        m
    };
    let reference = assert_walk_differential(
        "GPDMA mem2mem",
        GPDMA_FW_BASE as u32,
        STEPS,
        build,
        extra_gpdma,
    );

    let last = reference.last().unwrap();
    assert_eq!(last.extra("isr_count"), 1, "TC ISR must fire exactly once");
    assert!(last.extra("main_count") > 50, "main loop must run");
    assert_eq!(
        (last.extra("dst_lo"), last.extra("dst_hi")),
        expected_gpdma_dst(),
        "mem2mem must copy source bytes"
    );
    // Completion CSR: IDLEF | TCF | HTF = 0x301 (or 0x001 after ISR CFCR clear).
    assert!(
        reference.iter().any(|p| p.extra("csr") & 0x100 != 0),
        "CSR.TCF must latch at least once before the ISR clears it"
    );
}

/// Gate: same GPDMA memcpy firmware at the historical 512 interval and the
/// current recommendation, one batched `run` per lane. Relative delay-1
/// element chains pace identically in walk and scheduler at the shared batch
/// boundary, so final state must be byte-identical at both widths.
#[test]
fn gpdma_mem2mem_is_byte_identical_at_certified_intervals() {
    for interval in [512, RECOMMENDED_TICK_INTERVAL] {
        // 16 elements × interval cycles/tick + setup + ISR margin.
        let steps = u64::from(interval) * 32 + 4_000;

        let mut walk = build_gpdma_machine(false, interval);
        load_gpdma_memcpy(&mut walk.bus);
        walk.cpu.pc = GPDMA_FW_BASE as u32;
        walk.run(Some(steps as u32)).unwrap();
        let walk_probe = probe_at(&walk, steps, &extra_gpdma);

        let mut sched = build_gpdma_machine(true, interval);
        load_gpdma_memcpy(&mut sched.bus);
        sched.cpu.pc = GPDMA_FW_BASE as u32;
        sched.run(Some(steps as u32)).unwrap();
        let sched_probe = probe_at(&sched, steps, &extra_gpdma);

        assert_eq!(
            walk_probe, sched_probe,
            "interval-{interval} batched run: final state diverged (walk vs scheduler)"
        );
        assert_eq!(
            walk_probe.extra("isr_count"),
            1,
            "the TC ISR must fire once at interval {interval}"
        );
        assert_eq!(
            (walk_probe.extra("dst_lo"), walk_probe.extra("dst_hi")),
            expected_gpdma_dst(),
            "mem2mem copies the source bytes at interval {interval}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// RTC v3 calendar second + Alarm A
// ═══════════════════════════════════════════════════════════════════════════

/// Hand-built RTC base (away from GPDMA / SCB windows).
const RTC_BASE: u32 = 0x4000_2800;
const RTC_IRQ: u32 = 28;
const RTC_TR: u64 = RTC_BASE as u64;
const RTC_SR: u64 = (RTC_BASE + 0x50) as u64;
const RTC_SCR: u64 = (RTC_BASE + 0x5C) as u64;
const RTC_WPR: u64 = (RTC_BASE + 0x24) as u64;
const RTC_ICSR: u64 = (RTC_BASE + 0x0C) as u64;
const RTC_CR: u64 = (RTC_BASE + 0x18) as u64;
const RTC_ALRMAR: u64 = (RTC_BASE + 0x40) as u64;

const RTC_FW_BASE: u64 = 0x40;
const RTC_ISR_BASE: u64 = 0x80;

/// Initial TR = 12:34:56; Alarm A matches seconds == 57 (MSK2|MSK3|MSK4).
const RTC_TR_START: u32 = 0x0012_3456;
const RTC_ALRMAR_SEC57: u32 = 0x8080_8057;
/// CR: BYPSHAD | ALRAE | ALRAIE
const RTC_CR_ALARM: u32 = 0x20 | (1 << 8) | (1 << 12);

fn build_rtc_machine(scheduler: bool, tick_interval: u32) -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);

    bus.add_peripheral(
        "rtc",
        RTC_BASE as u64,
        0x400,
        Some(RTC_IRQ),
        Box::new(RtcV3::new()),
    );

    if !scheduler {
        let idx = bus.find_peripheral_index_by_name("rtc").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<RtcV3>()
            .unwrap()
            .force_legacy_walk();
    }

    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    machine.cpu.sp = INITIAL_SP;
    machine
}

/// Host-side WPR unlock + init-mode program of TR / ALRMAR / CR (identical in
/// both lanes). Arms the scheduler second-boundary chain on the write that
/// exits init (or the first non-init write); walk lane ignores harvests.
fn program_rtc_alarm(bus: &mut SystemBus) {
    bus.write_u32(RTC_WPR, 0xCA).unwrap();
    bus.write_u32(RTC_WPR, 0x53).unwrap();
    bus.write_u32(RTC_ICSR, 0x80).unwrap(); // enter INIT
    bus.write_u32(RTC_TR, RTC_TR_START).unwrap();
    bus.write_u32(RTC_ALRMAR, RTC_ALRMAR_SEC57).unwrap();
    bus.write_u32(RTC_CR, RTC_CR_ALARM).unwrap();
    bus.write_u32(RTC_ICSR, 0x00).unwrap(); // exit INIT → calendar runs
}

/// ISR @ 0x80: SCR = ALRAF (w1c), isr_count++, bx lr.
fn load_rtc_isr(bus: &mut SystemBus) {
    load_thumb(
        bus,
        RTC_ISR_BASE,
        &[
            0x4803, 0x2101, 0x6001, // r0=SCR_ADDR; r1=1; str
            0x4803, 0x6801, 0x3101, 0x6001, 0x4770,
        ],
    );
    write_word(bus, RTC_ISR_BASE + 0x10, RTC_SCR as u32);
    write_word(bus, RTC_ISR_BASE + 0x14, ISR_COUNT_ADDR as u32);
    write_word(bus, (16 + RTC_IRQ) as u64 * 4, (RTC_ISR_BASE as u32) | 1);
}

/// Firmware: enable RTC NVIC line, spin main_count++ / poll TR / poll SR.
///
/// PC-relative (base = Align(addr+4, 4)):
///   0x40: 4807 → 0x60  NVIC_ISER0
///   0x42: 4908 → 0x64  1<<IRQ
///   0x44: 6001
///   0x46: 4808 → 0x68  RTC_BASE
///   0x48: 4A08 → 0x6C  MAIN_COUNT   (Align(0x4C)=0x4C + 32 = 0x6C)
///   0x4A: 2300  movs r3, #0
/// loop @ 0x4C:
///   0x4C: 3301  adds r3, #1
///   0x4E: 6013  str r3, [r2]
///   0x50: 6804  ldr r4, [r0]        ; TR
///   0x52: 6D05  ldr r5, [r0, #0x50] ; SR
///   0x54: E7FA  b 0x4C              ; (0x4C - (0x54+4))/2 = -6 → 0xE7FA
fn load_rtc_poll_firmware(bus: &mut SystemBus) {
    load_thumb(
        bus,
        RTC_FW_BASE,
        &[
            0x4807, 0x4908, 0x6001, 0x4808, 0x4A08, 0x2300, 0x3301, 0x6013, 0x6804, 0x6D05, 0xE7FA,
        ],
    );
    write_word(bus, 0x60, NVIC_ISER0);
    write_word(bus, 0x64, 1 << RTC_IRQ);
    write_word(bus, 0x68, RTC_BASE);
    write_word(bus, 0x6C, MAIN_COUNT_ADDR as u32);
    load_rtc_isr(bus);
}

/// The caller-supplied observables the harness probe carries for the RTC
/// fixture: the two RAM counters, the calendar register and the status word.
fn extra_rtc(machine: &Machine<CortexM>) -> Vec<(&'static str, u64)> {
    vec![
        (
            "isr_count",
            u64::from(machine.bus.read_u32(ISR_COUNT_ADDR).unwrap()),
        ),
        (
            "main_count",
            u64::from(machine.bus.read_u32(MAIN_COUNT_ADDR).unwrap()),
        ),
        ("tr", u64::from(machine.bus.read_u32(RTC_TR).unwrap())),
        ("sr", u64::from(machine.bus.read_u32(RTC_SR).unwrap())),
    ]
}

/// Gate: RTC second advance + Alarm A latch/IRQ — walk-on vs scheduler at
/// tick interval 1, every instruction-boundary observable byte-identical.
///
/// Window covers > one full BCD second after setup so TR 0x…56 → 0x…57 and
/// SR.ALRAF / the alarm ISR are observed.
#[test]
fn rtc_second_and_alarm_is_byte_identical_at_interval_1() {
    // Setup instructions + one full second + ISR + trailing poll margin.
    const STEPS: u64 = TICKS_PER_SECOND as u64 + 8_000;

    let build = |mode: WalkMode| {
        let mut m = build_rtc_machine(!mode.is_legacy_walk(), 1);
        program_rtc_alarm(&mut m.bus);
        load_rtc_poll_firmware(&mut m.bus);
        m
    };
    let reference = assert_walk_differential(
        "RTC second/alarm",
        RTC_FW_BASE as u32,
        STEPS,
        build,
        extra_rtc,
    );

    let last = reference.last().unwrap();
    assert_eq!(
        last.extra("tr"),
        0x0012_3457,
        "calendar must advance one BCD second (got TR={:#010x})",
        last.extra("tr")
    );
    assert_eq!(
        last.extra("isr_count"),
        1,
        "Alarm A ISR must fire exactly once on the second match"
    );
    assert!(
        reference.iter().any(|p| p.extra("sr") & 1 != 0) || last.extra("isr_count") == 1,
        "SR.ALRAF must latch (or already been cleared by the ISR)"
    );
    assert!(last.extra("main_count") > 100, "main loop must run");
}

/// Gate: scheduler at the historical 512 interval and the current
/// recommendation vs a walk-on interval-1 golden reference. Absolute second
/// deadlines make the observed second count and alarm ISR count exact.
#[test]
fn rtc_second_count_is_exact_at_certified_intervals() {
    // Two full seconds of calendar time after a short setup budget.
    const STEPS: u64 = 2 * TICKS_PER_SECOND as u64 + 4_000;

    let mut walk = build_rtc_machine(false, 1);
    program_rtc_alarm(&mut walk.bus);
    load_rtc_poll_firmware(&mut walk.bus);
    let walk_probes = run_probed(&mut walk, RTC_FW_BASE as u32, STEPS, &extra_rtc);

    let reference = walk_probes.last().unwrap();

    // Count TR second-field transitions in the reference (each BCD second).
    let mut walk_seconds = 0u32;
    for w in walk_probes.windows(2) {
        if w[1].extra("tr") != w[0].extra("tr") {
            walk_seconds += 1;
        }
    }

    // Last second observation must sit well clear of the window edge so
    // ≤-one-interval quantisation cannot drop a count at interval 512.
    let last_second_step = walk_probes
        .windows(2)
        .filter(|w| w[1].extra("tr") != w[0].extra("tr"))
        .map(|w| w[1].step)
        .next_back()
        .expect("reference must observe at least one second advance");
    assert!(
        STEPS - last_second_step > 600,
        "fixture must keep the window edge > interval + poll lag from the last \
         second observation (last at step {last_second_step})"
    );

    assert!(
        walk_seconds >= 2,
        "reference must cross at least two second boundaries (got {walk_seconds})"
    );
    for interval in [512, RECOMMENDED_TICK_INTERVAL] {
        let mut sched = build_rtc_machine(true, interval);
        program_rtc_alarm(&mut sched.bus);
        load_rtc_poll_firmware(&mut sched.bus);
        sched.cpu.pc = RTC_FW_BASE as u32;
        sched.run(Some(STEPS as u32)).unwrap();

        let sched_tr = sched.bus.read_u32(RTC_TR).unwrap();
        let sched_isr = sched.bus.read_u32(ISR_COUNT_ADDR).unwrap();
        assert_eq!(
            reference.extra("tr") as u32,
            sched_tr,
            "final TR must match at sched@{interval}: walk={:#010x} sched={:#010x}",
            reference.extra("tr"),
            sched_tr
        );
        // Alarm matches only second==57 (one fire); later seconds do not
        // re-match until TR returns to :57.
        assert_eq!(
            reference.extra("isr_count") as u32,
            sched_isr,
            "Alarm A ISR count must be exact at interval {interval}"
        );
        assert_eq!(
            sched.total_cycles, reference.total_cycles,
            "total_cycles over the fixed instruction window must match at interval {interval}"
        );
    }
}
